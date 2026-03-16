//! Unified 3-column browser TUI for searching dependencies, packages, and symbols.
//!
//! Provides a Miller-columns style interface where `Deps → Packages → Symbols`
//! are displayed side by side.  The user navigates between columns with
//! arrow keys and can drill into source code via an overlay.

mod browser;

use std::path::Path;

use anyhow::Result;

use crate::model::{AccessLevel, SymbolKind};

/// Which column should receive initial focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnFocus {
    /// Dependencies column.
    #[default]
    Dep,
    /// Packages column.
    Pkg,
    /// Symbols column.
    Symbol,
}

/// Configuration for launching the unified browser.
///
/// Maps directly from the CLI `search {dep,pkg,symbol}` subcommands so that
/// each entry point pre-selects the appropriate column and filters.
#[derive(Debug, Default)]
pub struct BrowserConfig<'a> {
    /// Which column gets focus on launch.
    pub initial_focus: ColumnFocus,
    /// GAV glob pattern filter for the deps column.
    pub dep_query: Option<&'a str>,
    /// Package glob pattern filter for the packages column.
    pub pkg_query: Option<&'a str>,
    /// Symbol name/FQN filter for the symbols column.
    pub symbol_query: Option<&'a str>,
    /// Classpath filter (e.g. `"compile"`, `"runtime"`).
    pub classpath: Option<&'a str>,
    /// Symbol type filters.
    pub symbol_types: &'a [SymbolKind],
    /// Access level filters.
    pub access_levels: &'a [AccessLevel],
}

/// Launch the unified 3-column browser TUI.
///
/// Opens the index at `project_dir` and enters an interactive event loop.
/// The `config` determines which column gets initial focus and what filters
/// are pre-applied.
pub fn run(project_dir: &Path, config: &BrowserConfig) -> Result<()> {
    browser::run(project_dir, config)
}
