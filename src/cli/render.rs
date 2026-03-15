//! Plain-text renderers for CLI output.
//!
//! These functions reproduce the original ASCII-table and text output that
//! was previously inlined in each handler, used when stdout is not a TTY.

use crate::model::{
    CleanOutput, DepsOutput, InitOutput, RefreshOutput, SearchOutput, ShowOutput, StatusOutput,
    format_lang_display,
};

/// Render search results as a compact list (used when listing symbols without a query).
pub fn search_list(output: &SearchOutput) {
    let pattern = output.dependency.as_deref().unwrap_or("*");

    if output.results.is_empty() {
        println!("No symbols found for '{pattern}'.");
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
        println!("[{}] {}  {}", sym.symbol_kind, sym.fqn, sym.signature.java);
    }
}

/// Render search results as a plain-text ASCII table.
pub fn search(output: &SearchOutput) {
    let query_display = output.query.as_deref().unwrap_or("*");

    if output.results.is_empty() {
        println!("No results found for '{}'.", query_display);
        return;
    }

    // Show truncation notice if there are more matches than displayed
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

    let has_kotlin = output.results.iter().any(|r| r.signature.kotlin.is_some());
    let any_source = output.results.iter().any(|r| r.has_source());
    let any_scopes = output.results.iter().any(|r| !r.scopes.is_empty());

    // Column widths
    let w_fqn = output
        .results
        .iter()
        .map(|r| r.fqn.len())
        .max()
        .unwrap_or(6)
        .clamp(6, 60);
    let w_sig = output
        .results
        .iter()
        .map(|r| r.signature.java.len())
        .max()
        .unwrap_or(14)
        .clamp(14, 80);
    let w_kt = if has_kotlin {
        output
            .results
            .iter()
            .filter_map(|r| r.signature.kotlin.as_ref())
            .map(|s| s.len())
            .max()
            .unwrap_or(16)
            .clamp(16, 80)
    } else {
        0
    };
    let w_src = 10; // "Decompiler"
    let w_lang = 7; // "Clojure"
    let w_scope = if any_scopes {
        output
            .results
            .iter()
            .map(|r| format_scopes(&r.scopes).len())
            .max()
            .unwrap_or(5)
            .clamp(5, 30)
    } else {
        0
    };

    // Header
    print!("{:<w_fqn$}  {:<w_sig$}", "Symbol", "Java Signature");
    if has_kotlin {
        print!("  {:<w_kt$}", "Kotlin Signature");
    }
    print!("  {:<w_src$}", "Src");
    if any_source {
        print!("  {:<w_lang$}", "Lang");
    }
    print!("  Dependency");
    if any_scopes {
        print!("  {:<w_scope$}", "Scope");
    }
    println!();

    // Separator
    print!("{:-<w_fqn$}  {:-<w_sig$}", "", "");
    if has_kotlin {
        print!("  {:-<w_kt$}", "");
    }
    print!("  {:-<w_src$}", "");
    if any_source {
        print!("  {:-<w_lang$}", "");
    }
    print!("  {:-<20}", "");
    if any_scopes {
        print!("  {:-<w_scope$}", "");
    }
    println!();

    // Rows
    for r in &output.results {
        let fqn_display = truncate_or_pad(&r.fqn, w_fqn);
        let sig_display = truncate_or_pad(&r.signature.java, w_sig);

        print!("{:<w_fqn$}  {:<w_sig$}", fqn_display, sig_display);

        if has_kotlin {
            let kt_display = truncate_or_pad(r.signature.kotlin.as_deref().unwrap_or(""), w_kt);
            print!("  {:<w_kt$}", kt_display);
        }

        print!(
            "  {:<w_src$}",
            if r.has_source() {
                "Source"
            } else {
                "Decompiled"
            }
        );

        if any_source {
            let lang = if r.has_source() {
                let l = r.source_language.map(|l| l.to_string());
                format_lang_display(l.as_deref().unwrap_or("java"))
            } else {
                ""
            };
            print!("  {:<w_lang$}", lang);
        }

        print!("  {}", r.gav);
        if any_scopes {
            print!("  {:<w_scope$}", format_scopes(&r.scopes));
        }
        println!();
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
        println!("Not initialized. Run `classpath-surfer init` first.");
        return;
    }

    if !output.has_index && output.dependency_count == 0 {
        println!("No index built. Run `classpath-surfer refresh` to build it.");
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
    println!(
        "Refresh complete: {} mode, {} dependencies processed, {} symbols indexed.",
        output.mode, output.dependencies_processed, output.symbols_indexed
    );
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
            println!("  Removed: {item}");
        }
        println!("Clean complete.");
    }
}

/// Render dependency list as plain text.
pub fn deps(output: &DepsOutput) {
    if output.dependencies.is_empty() {
        if let Some(ref filter) = output.filter {
            println!("No dependencies matching '{filter}'.");
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
        let scope_info = format_scopes(&dep.scopes);
        if scope_info.is_empty() {
            println!("{} ({} symbols)", dep.gav, dep.symbol_count);
        } else {
            println!(
                "{} ({} symbols) [{}]",
                dep.gav, dep.symbol_count, scope_info
            );
        }
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

fn truncate_or_pad(s: &str, max_width: usize) -> String {
    if s.len() > max_width {
        if max_width > 3 {
            format!("{}...", &s[..max_width - 3])
        } else {
            s[..max_width].to_string()
        }
    } else {
        s.to_string()
    }
}
