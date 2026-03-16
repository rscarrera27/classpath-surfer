//! Structured output DTOs for CLI commands.
//!
//! Each command handler returns an `XxxOutput` struct that is serialized to JSON
//! in agentic mode or rendered as plain text / TUI otherwise.

use serde::Serialize;

use super::{AccessLevel, SearchResult, SourceOrigin, SymbolKind};

/// Query parameters for symbol search.
///
/// Bundles all search options into a single struct to avoid long argument
/// lists in [`crate::cli::search::run`] and [`crate::index::reader::IndexReader::search`].
///
/// Either `query` or `dependency` (or both) must be provided.  When `query`
/// is `None` and `dependency` is set, all symbols for the matching
/// dependencies are returned (sorted by kind then FQN).
#[derive(Debug)]
pub struct SearchQuery<'a> {
    /// Symbol name, FQN, or regex pattern.  `None` to list all symbols
    /// (requires `dependency` to be set).
    pub query: Option<&'a str>,
    /// Filter by symbol kind.  Empty slice = any (no filter).
    pub symbol_types: &'a [SymbolKind],
    /// Exact FQN match mode.
    pub fqn_mode: bool,
    /// Regex search mode.
    pub regex_mode: bool,
    /// Maximum number of results.
    pub limit: usize,
    /// Number of results to skip (for pagination).
    pub offset: usize,
    /// Restrict to dependencies matching a GAV pattern (glob, e.g. `"com.google.*:guava:*"`).
    pub dependency: Option<&'a str>,
    /// Filter by access level.  Empty slice = all (no filter).
    pub access_levels: &'a [AccessLevel],
    /// Filter results to a specific configuration scope (e.g. `"compileClasspath"`).
    pub scope: Option<&'a str>,
    /// Filter results by Java package pattern (glob with `*` wildcards, e.g. `"com.google.common.*"`).
    pub package: Option<&'a str>,
}

impl<'a> SearchQuery<'a> {
    /// Create a search query with default options.
    pub fn simple(query: &'a str) -> Self {
        Self {
            query: Some(query),
            symbol_types: &[],
            fqn_mode: false,
            regex_mode: false,
            limit: 20,
            offset: 0,
            dependency: None,
            access_levels: &[],
            scope: None,
            package: None,
        }
    }

    /// Create a search query filtered to specific symbol types.
    pub fn with_types(query: &'a str, symbol_types: &'a [SymbolKind]) -> Self {
        Self {
            symbol_types,
            ..Self::simple(query)
        }
    }
}

/// Structured output for the `search` command.
#[derive(Debug, Serialize)]
pub struct SearchOutput {
    /// The original query string (`None` when listing all symbols for a dependency).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// GAV pattern used to filter dependencies (`None` when searching all dependencies).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency: Option<String>,
    /// Package pattern used to filter results (`None` when no package filter was applied).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// GAVs that matched the dependency pattern (`None` when no pattern was used).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_gavs: Option<Vec<String>>,
    /// Total number of matching documents (may exceed `results.len()` due to limit).
    pub total_matches: usize,
    /// Offset used for this page of results.
    pub offset: usize,
    /// Limit used for this page of results.
    pub limit: usize,
    /// Whether more results are available beyond this page.
    pub has_more: bool,
    /// Matching symbols ranked by relevance.
    pub results: Vec<SearchResult>,
}

/// Structured output for the `show` command.
#[derive(Debug, Serialize)]
pub struct ShowOutput {
    /// Fully qualified name of the displayed symbol.
    pub fqn: String,
    /// Maven GAV coordinates of the dependency containing this class.
    pub gav: String,
    /// The symbol's simple name, if showing a member (method/field).
    /// `None` when showing a full class.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_name: Option<String>,
    /// Primary source view (source or decompiler).
    pub primary: SourceView,
    /// Optional secondary view (e.g. decompiler Java for Kotlin sources).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<SourceView>,
}

/// Focus metadata for windowed source display.
///
/// Used internally by TUI (scroll) and plain renderer (line numbering).
/// Not included in JSON output — `source_path#L` fragment serves that role.
#[derive(Debug)]
pub struct FocusInfo {
    /// 1-based line number of the symbol definition in the original source.
    pub symbol_line: usize,
    /// 1-based start line of the displayed window.
    pub start_line: usize,
    /// 1-based end line of the displayed window (inclusive).
    pub end_line: usize,
    /// Total number of lines in the original source file.
    pub total_lines: usize,
}

/// A single source-code view within [`ShowOutput`].
#[derive(Debug, Serialize)]
pub struct SourceView {
    /// Source file content.
    pub content: String,
    /// Language identifier (`"java"`, `"kotlin"`, `"scala"`, `"groovy"`, `"clojure"`, or `"unknown"`).
    pub language: String,
    /// Source origin (source JAR with path, or decompiled).
    #[serde(flatten)]
    pub source: SourceOrigin,
    /// Number of lines in the source content.
    pub line_count: usize,
    /// Focus metadata — used internally by TUI (scroll position) and plain
    /// renderer (line numbering).  Not serialized to JSON; the `source_path`
    /// `#L` fragment already encodes the visible range for external consumers.
    #[serde(skip)]
    pub focus: Option<FocusInfo>,
}

/// Structured output for the `status` command.
#[derive(Debug, Serialize)]
pub struct StatusOutput {
    /// Whether `.classpath-surfer/` directory exists.
    pub initialized: bool,
    /// Whether the Tantivy index directory exists.
    pub has_index: bool,
    /// Total number of resolved dependencies.
    pub dependency_count: usize,
    /// Dependencies that have a source JAR.
    pub with_source_jars: usize,
    /// Dependencies without a source JAR.
    pub without_source_jars: usize,
    /// Number of indexed symbols (if index exists).
    pub indexed_symbols: Option<usize>,
    /// Whether the index is stale (dependencies changed).
    pub is_stale: bool,
    /// Human-readable index size on disk.
    pub index_size: Option<String>,
}

/// Structured output for the `refresh` command.
#[derive(Debug, Serialize)]
pub struct RefreshOutput {
    /// Refresh mode: `"full"`, `"incremental"`, or `"up_to_date"`.
    pub mode: String,
    /// Number of dependencies processed.
    pub dependencies_processed: usize,
    /// Total number of symbols indexed in this run.
    pub symbols_indexed: usize,
}

/// Structured output for the `init` command.
#[derive(Debug, Serialize)]
pub struct InitOutput {
    /// Descriptions of actions performed during initialization.
    pub actions: Vec<String>,
}

/// Structured output for the `clean` command.
#[derive(Debug, Serialize)]
pub struct CleanOutput {
    /// Descriptions of items that were removed.
    pub items_removed: Vec<String>,
}

/// Structured output for the `pkgs` command.
#[derive(Debug, Serialize)]
pub struct PkgsOutput {
    /// Filter pattern applied (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// GAV pattern used to restrict to specific dependencies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency: Option<String>,
    /// GAVs that matched the dependency pattern.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_gavs: Option<Vec<String>>,
    /// Total number of packages matching the filter.
    pub total_count: usize,
    /// Offset used for this page of results.
    pub offset: usize,
    /// Limit used for this page of results.
    pub limit: usize,
    /// Whether more results are available beyond this page.
    pub has_more: bool,
    /// Package entries with symbol counts.
    pub packages: Vec<PkgInfo>,
}

/// A single package entry in [`PkgsOutput`].
#[derive(Debug, Serialize)]
pub struct PkgInfo {
    /// Java package name (e.g. `com.google.common.collect`).
    pub package: String,
    /// Number of indexed symbols in this package.
    pub symbol_count: usize,
}

/// Structured output for the `deps` command.
#[derive(Debug, Serialize)]
pub struct DepsOutput {
    /// Filter pattern applied (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Total number of dependencies matching the filter.
    pub total_count: usize,
    /// Offset used for this page of results.
    pub offset: usize,
    /// Limit used for this page of results.
    pub limit: usize,
    /// Whether more results are available beyond this page.
    pub has_more: bool,
    /// Dependency entries with symbol counts.
    pub dependencies: Vec<DepInfo>,
}

/// A single dependency entry in [`DepsOutput`].
#[derive(Debug, Serialize)]
pub struct DepInfo {
    /// Maven GAV coordinates (`group:artifact:version`).
    pub gav: String,
    /// Number of indexed symbols in this dependency.
    pub symbol_count: usize,
    /// Configuration scopes that include this dependency.
    pub scopes: Vec<String>,
}
