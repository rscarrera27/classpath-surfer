use std::path::Path;

use anyhow::Result;

use crate::manifest::ClasspathManifest;
use crate::model::{ShowOutput, SourceOrigin, SourceProvider, SourceView};
use crate::source::resolver;

/// Retrieve source code for a fully qualified name and return structured output.
///
/// Tries the source JAR first; if unavailable, falls back to decompilation
/// (unless `no_decompile` is set).
pub fn run(
    project_dir: &Path,
    fqn: &str,
    decompiler: &str,
    decompiler_jar: Option<&Path>,
    no_decompile: bool,
) -> Result<ShowOutput> {
    super::require_manifest(project_dir)?;

    let manifest_path = project_dir.join(".classpath-surfer/classpath-manifest.json");
    let manifest: ClasspathManifest = load_manifest(&manifest_path)?;

    load_show_output(
        project_dir,
        &manifest,
        fqn,
        decompiler,
        decompiler_jar,
        no_decompile,
    )
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
    decompiler: &str,
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
        primary,
        secondary,
    })
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
            }
        }
    }
}
