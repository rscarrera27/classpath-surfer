//! Plain-text renderers for CLI output.
//!
//! These functions produce TAB-separated, machine-parseable output for use in
//! pipes and non-TTY environments. Each line is a single record with fixed
//! columns separated by `\t`, suitable for `cut`, `awk`, `sort`, and `grep`.

use crate::model::{
    CleanOutput, DepsOutput, InitOutput, PkgsOutput, RefreshOutput, SearchOutput, ShowOutput,
    StatusOutput, format_lang_display,
};

/// Render search results as a compact list (used when listing symbols without a query).
///
/// Columns: `FQN\tKIND\tSIGNATURE\tGAV`
pub fn search_list(output: &SearchOutput) {
    let pattern = output.dependency.as_deref().unwrap_or("*");

    if output.results.is_empty() {
        if let Some(ref pkg) = output.package {
            println!("No symbols found for '{pattern}' in package '{pkg}'.");
        } else {
            println!("No symbols found for '{pattern}'.");
        }
        return;
    }

    if output.has_more {
        eprintln!(
            "Showing {} of {} symbols. Use --offset {} to see more.",
            output.results.len(),
            output.total_matches,
            output.offset + output.results.len()
        );
    }

    for sym in &output.results {
        println!(
            "{}\t{}\t{}\t{}",
            sym.fqn, sym.symbol_kind, sym.signature.java, sym.gav
        );
    }
}

/// Render search results as TAB-separated plain text.
///
/// Columns: `FQN\tKIND\tJAVA_SIG\tKOTLIN_SIG\tSOURCE\tLANG\tGAV\tSCOPE`
pub fn search(output: &SearchOutput) {
    let query_display = output.query.as_deref().unwrap_or("*");

    if output.results.is_empty() {
        if let Some(ref pkg) = output.package {
            println!(
                "No matches found for '{}' in package '{}'.",
                query_display, pkg
            );
        } else {
            println!("No matches found for '{}'.", query_display);
        }
        return;
    }

    if output.has_more {
        eprintln!(
            "Showing {} of {} matches for '{}'. Use --offset {} to see more.",
            output.results.len(),
            output.total_matches,
            query_display,
            output.offset + output.results.len()
        );
    } else if output.total_matches > output.results.len() {
        eprintln!(
            "Showing {} of {} matches for '{}'.",
            output.results.len(),
            output.total_matches,
            query_display
        );
    }

    for r in &output.results {
        let kt_sig = r.signature.kotlin.as_deref().unwrap_or("");
        let source = if r.has_source() {
            "source"
        } else {
            "decompiled"
        };
        let lang = if r.has_source() {
            let l = r.source_language.map(|l| l.to_string());
            format_lang_display(l.as_deref().unwrap_or("java"))
        } else {
            ""
        };
        let scope = format_scopes(&r.scopes);
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            r.fqn, r.symbol_kind, r.signature.java, kt_sig, source, lang, r.gav, scope
        );
    }
}

/// Render source code as plain text with header comments.
pub fn show(output: &ShowOutput) {
    let lang = format_lang_display(output.primary.language.as_str());
    let source_label = if output.primary.source.has_source() {
        format!(
            "Source ({}): {}",
            lang,
            output.primary.source.source_path().unwrap_or("unknown")
        )
    } else {
        format!("Decompiled ({lang}, no source JAR available)")
    };
    eprintln!("// {source_label}");
    eprintln!("// GAV: {}", output.gav);

    if let Some(focus) = &output.primary.focus {
        let name = output.symbol_name.as_deref().unwrap_or(&output.fqn);
        eprintln!(
            "// Lines {}-{} of {} (focused on '{name}')",
            focus.start_line, focus.end_line, focus.total_lines,
        );
        for (i, line) in output.primary.content.lines().enumerate() {
            let line_num = focus.start_line + i;
            println!("{line_num:>5} | {line}");
        }
    } else {
        println!("{}", output.primary.content);
    }

    if let Some(secondary) = &output.secondary {
        eprintln!();
        eprintln!("// Decompiled Java (secondary view):");
        println!("{}", secondary.content);
    }
}

/// Render index status as plain text.
pub fn status(output: &StatusOutput) {
    if !output.initialized {
        println!("Not initialized. Run `classpath-surfer index init` first.");
        return;
    }

    if !output.has_index && output.dependency_count == 0 {
        println!("No index built. Run `classpath-surfer index refresh` to build it.");
        return;
    }

    println!("Dependencies: {}", output.dependency_count);
    println!("  with source JARs: {}", output.with_source_jars);
    println!("  without source JARs: {}", output.without_source_jars);

    if let Some(count) = output.indexed_symbols {
        println!("Indexed symbols: {count}");
    } else {
        println!("Index: not built");
    }

    println!("Stale: {}", if output.is_stale { "yes" } else { "no" });

    if let Some(ref size) = output.index_size {
        println!("Index size: {size}");
    }
}

/// Render refresh summary as plain text.
pub fn refresh(output: &RefreshOutput) {
    if output.mode == "up_to_date" {
        println!("Nothing to refresh. Index is up to date.");
    } else {
        println!(
            "Refresh complete: {} mode, {} dependencies processed, {} symbols indexed.",
            output.mode, output.dependencies_processed, output.symbols_indexed
        );
    }
}

/// Render init summary as plain text.
pub fn init(output: &InitOutput) {
    for action in &output.actions {
        println!("  {action}");
    }
    println!("Initialization complete.");
}

/// Render clean summary as plain text.
pub fn clean(output: &CleanOutput) {
    if output.items_removed.is_empty() {
        println!("Nothing to clean.");
    } else {
        for item in &output.items_removed {
            println!("  {item}");
        }
        println!("Clean complete.");
    }
}

/// Render dependency list as TAB-separated plain text.
///
/// Columns: `GAV\tSYMBOL_COUNT\tSCOPE`
pub fn deps(output: &DepsOutput) {
    if output.dependencies.is_empty() {
        if let Some(ref q) = output.query {
            println!("No dependencies matching '{q}'.");
        } else {
            println!("No dependencies found.");
        }
        return;
    }

    if output.has_more {
        eprintln!(
            "Showing {} of {} dependencies. Use --offset {} to see more.",
            output.dependencies.len(),
            output.total_count,
            output.offset + output.dependencies.len()
        );
    }

    for dep in &output.dependencies {
        println!(
            "{}\t{}\t{}",
            dep.gav,
            dep.symbol_count,
            format_scopes(&dep.scopes)
        );
    }
}

/// Render package list as TAB-separated plain text.
///
/// Columns: `PACKAGE\tSYMBOL_COUNT`
pub fn pkgs(output: &PkgsOutput) {
    if output.packages.is_empty() {
        match (&output.query, &output.dependency) {
            (Some(q), Some(d)) => println!("No packages matching '{q}' in dependency '{d}'."),
            (Some(q), None) => println!("No packages matching '{q}'."),
            (None, Some(d)) => println!("No packages found in dependency '{d}'."),
            (None, None) => println!("No packages found."),
        }
        return;
    }

    if output.has_more {
        eprintln!(
            "Showing {} of {} packages. Use --offset {} to see more.",
            output.packages.len(),
            output.total_count,
            output.offset + output.packages.len()
        );
    }

    for pkg in &output.packages {
        println!("{}\t{}", pkg.package, pkg.symbol_count);
    }
}

/// Format configuration scopes for display (e.g. "compile+runtime").
fn format_scopes(scopes: &[String]) -> String {
    if scopes.is_empty() {
        return String::new();
    }
    scopes
        .iter()
        .map(|s| s.strip_suffix("Classpath").unwrap_or(s))
        .collect::<Vec<_>>()
        .join("+")
}
