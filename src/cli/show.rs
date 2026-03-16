use std::path::Path;

use anyhow::Result;

use crate::index::reader::IndexReader;
use crate::manifest::ClasspathManifest;
use crate::model::{
    FocusInfo, SearchQuery, ShowOutput, SourceOrigin, SourceProvider, SourceView, SymbolKind,
};
use crate::parser::jar;
use crate::source::decompiler::Decompiler;
use crate::source::{locator, resolver};

/// Show options controlling focus behavior.
pub struct ShowOptions<'a> {
    /// Fully qualified name (class, method, or field).
    pub fqn: &'a str,
    /// Decompiler backend.
    pub decompiler: Decompiler,
    /// Path to decompiler JAR.
    pub decompiler_jar: Option<&'a Path>,
    /// Fail instead of decompiling.
    pub no_decompile: bool,
    /// Context lines before/after symbol (for method FQNs with source JAR).
    pub context: usize,
    /// Force full file display (focus metadata still set for TUI scroll).
    pub full: bool,
}

/// Minimum gap between window size and total lines before windowing kicks in.
const FOCUS_MIN_SAVINGS: usize = 10;

/// Lines above the symbol when centering in TUI viewport.
pub const FOCUS_TOP_MARGIN: u16 = 5;

/// Retrieve source code for a symbol and return structured output.
///
/// Supports class, method, and field FQNs. For methods and fields with a
/// source JAR, focuses the output around the symbol definition using
/// `LineNumberTable` (fields use their getter method's line number as proxy).
pub fn run(project_dir: &Path, opts: &ShowOptions<'_>) -> Result<ShowOutput> {
    super::require_manifest(project_dir)?;

    let manifest_path = project_dir.join(".classpath-surfer/classpath-manifest.json");
    let manifest: ClasspathManifest = load_manifest(&manifest_path)?;

    run_with_manifest(project_dir, &manifest, opts)
}

/// Load show output with optional symbol focusing.
///
/// Resolves member FQNs via the index and applies focus windowing.
/// Used by TUI search for auto-scroll.
pub fn load_show_output_focused(
    project_dir: &Path,
    manifest: &ClasspathManifest,
    opts: &ShowOptions<'_>,
) -> Result<ShowOutput> {
    run_with_manifest(project_dir, manifest, opts)
}

fn run_with_manifest(
    project_dir: &Path,
    manifest: &ClasspathManifest,
    opts: &ShowOptions<'_>,
) -> Result<ShowOutput> {
    // 1. Index lookup -> symbol_kind + simple_name
    let member_info = lookup_member_info(project_dir, opts.fqn);

    // 2. Split into class FQN + optional member
    let (class_fqn, member) = match &member_info {
        Some((kind, simple_name)) if *kind != SymbolKind::Class => {
            let class_fqn = opts
                .fqn
                .strip_suffix(&format!(".{simple_name}"))
                .unwrap_or(opts.fqn);
            (class_fqn.to_string(), Some((simple_name.as_str(), *kind)))
        }
        _ => (opts.fqn.to_string(), None),
    };

    // 3. Resolve source for the class
    let mut output = load_show_output(
        project_dir,
        manifest,
        &class_fqn,
        opts.decompiler,
        opts.decompiler_jar,
        opts.no_decompile,
    )?;
    output.fqn = opts.fqn.to_string();

    // 4. Focus for methods and fields with source
    if let Some((simple_name, kind)) = member
        && output.primary.source.has_source()
    {
        output.symbol_name = Some(simple_name.to_string());

        let symbol_line = match kind {
            SymbolKind::Method => {
                resolve_method_line(manifest, &output.gav, &class_fqn, simple_name)
            }
            SymbolKind::Field => resolve_field_line(manifest, &output.gav, &class_fqn, simple_name),
            _ => None,
        };

        if opts.full {
            set_focus_metadata(&mut output.primary, symbol_line);
        } else {
            apply_focus(&mut output.primary, symbol_line, opts.context);
        }
    }

    Ok(output)
}

/// Load a [`ShowOutput`] without performing staleness checks.
///
/// This is the core source-resolution logic shared by the CLI `show` handler
/// and the TUI search viewer (which loads the manifest once and skips repeated
/// staleness checks on every Enter press).
pub fn load_show_output(
    project_dir: &Path,
    manifest: &ClasspathManifest,
    fqn: &str,
    decompiler: Decompiler,
    decompiler_jar: Option<&Path>,
    no_decompile: bool,
) -> Result<ShowOutput> {
    let resolved = resolver::resolve_source(
        fqn,
        project_dir,
        manifest,
        decompiler,
        decompiler_jar,
        no_decompile,
    )?;

    let primary = source_code_to_view(&resolved.primary);
    let secondary = resolved.secondary.as_ref().map(source_code_to_view);

    Ok(ShowOutput {
        fqn: fqn.to_string(),
        gav: resolved.gav,
        symbol_name: None,
        primary,
        secondary,
    })
}

/// Extract class bytes for a given GAV and class FQN.
fn extract_class_bytes(
    manifest: &ClasspathManifest,
    gav: &str,
    class_fqn: &str,
) -> Option<Vec<u8>> {
    let class_path = resolver::fqn_to_class_path(class_fqn);
    let dep = manifest
        .all_dependencies()
        .into_iter()
        .find(|d| d.gav() == gav)?;
    jar::extract_entry(&dep.jar_path, &class_path).ok()
}

/// Extract method line from classfile `LineNumberTable`.
fn resolve_method_line(
    manifest: &ClasspathManifest,
    gav: &str,
    class_fqn: &str,
    simple_name: &str,
) -> Option<usize> {
    let class_bytes = extract_class_bytes(manifest, gav, class_fqn)?;

    // Constructors: index stores class simple name (with $ replaced by .),
    // classfile uses "<init>". Normalize $ → . for comparison.
    let class_simple = class_fqn.rsplit('.').next().unwrap_or(class_fqn);
    let class_simple_normalized = class_simple.replace('$', ".");
    let classfile_name = if simple_name == class_simple_normalized {
        "<init>"
    } else {
        simple_name
    };

    locator::find_method_line_from_classfile(&class_bytes, classfile_name)
}

/// Extract field line from the classfile by finding its getter method's `LineNumberTable`.
///
/// Fields themselves have no line number info, but Kotlin properties generate
/// getter methods (`getXxx`) whose `LineNumberTable` points to the property
/// declaration line.
fn resolve_field_line(
    manifest: &ClasspathManifest,
    gav: &str,
    class_fqn: &str,
    simple_name: &str,
) -> Option<usize> {
    let class_bytes = extract_class_bytes(manifest, gav, class_fqn)?;

    // Try getter method (Kotlin property: val foo → getFoo())
    let getter = format!(
        "get{}",
        simple_name
            .chars()
            .next()
            .map(|c| {
                let upper: String = c.to_uppercase().collect();
                format!("{upper}{}", &simple_name[c.len_utf8()..])
            })
            .unwrap_or_default()
    );
    if let Some(line) = locator::find_method_line_from_classfile(&class_bytes, &getter) {
        return Some(line);
    }

    // Try field name directly (Kotlin boolean `isXxx` properties keep the name as-is)
    locator::find_method_line_from_classfile(&class_bytes, simple_name)
}

/// Set focus metadata without truncating content (--full / TUI mode).
fn set_focus_metadata(view: &mut SourceView, symbol_line: Option<usize>) {
    if let Some(symbol_line) = symbol_line {
        let total_lines = view.content.lines().count();
        view.focus = Some(FocusInfo {
            symbol_line,
            start_line: 1,
            end_line: total_lines,
            total_lines,
        });
    }
}

/// Window the source content around the method definition.
fn apply_focus(view: &mut SourceView, symbol_line: Option<usize>, context: usize) {
    let symbol_line = match symbol_line {
        Some(l) => l,
        None => return,
    };

    let total_lines = view.content.lines().count();
    let window_size = context * 2 + 1;
    if total_lines <= window_size + FOCUS_MIN_SAVINGS {
        view.focus = Some(FocusInfo {
            symbol_line,
            start_line: 1,
            end_line: total_lines,
            total_lines,
        });
        return;
    }

    let start = symbol_line.saturating_sub(context).max(1);
    let end = (symbol_line + context).min(total_lines);

    let focused_content: String = view
        .content
        .lines()
        .enumerate()
        .filter(|(i, _)| {
            let n = i + 1;
            n >= start && n <= end
        })
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n");

    view.focus = Some(FocusInfo {
        symbol_line,
        start_line: start,
        end_line: end,
        total_lines,
    });
    view.content = focused_content;
    view.line_count = end - start + 1;

    // Append #L fragment to source_path
    if let SourceOrigin::SourceJar {
        source_path: Some(ref mut path),
        ..
    } = view.source
    {
        *path = format!("{path}#L{start}-L{end}");
    }
}

/// Look up FQN in the index to determine symbol_kind and simple_name.
///
/// FQNs always have 2+ dots, so the auto-detect logic in `build_base_query`
/// treats them as exact FQN matches — no explicit mode flag needed.
fn lookup_member_info(project_dir: &Path, fqn: &str) -> Option<(SymbolKind, String)> {
    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir).ok()?;
    let query = SearchQuery {
        query: Some(fqn),
        symbol_types: &[],
        limit: 1,
        offset: 0,
        dependency: None,
        access_levels: &[],
        scope: None,
        package: None,
    };
    let (results, _, _) = reader.search(&query).ok()?;
    let result = results.first()?;
    Some((result.symbol_kind, result.simple_name.clone()))
}

/// Load and parse a [`ClasspathManifest`] from disk.
pub fn load_manifest(manifest_path: &Path) -> Result<ClasspathManifest> {
    let json = std::fs::read_to_string(manifest_path)?;
    Ok(serde_json::from_str(&json)?)
}

fn source_code_to_view(source: &SourceProvider) -> SourceView {
    match source {
        SourceProvider::SourceJar {
            content,
            path,
            language,
        } => {
            let line_count = content.lines().count();
            SourceView {
                content: content.clone(),
                language: language.to_string(),
                source: SourceOrigin::SourceJar {
                    source_path: Some(path.clone()),
                    source_language: Some(*language),
                    source_file_name: None,
                },
                line_count,
                focus: None,
            }
        }
        SourceProvider::Decompiler { content } => {
            let line_count = content.lines().count();
            SourceView {
                content: content.clone(),
                language: "java".to_string(),
                source: SourceOrigin::Decompiled {
                    source_language: None,
                    source_file_name: None,
                },
                line_count,
                focus: None,
            }
        }
    }
}
