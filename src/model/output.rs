//! Structured output DTOs for CLI commands.
//!
//! Each command handler returns an `XxxOutput` struct that is serialized to JSON
//! in agentic mode or rendered as plain text / TUI otherwise.

use serde::Serialize;

use super::{SearchResult, SourceOrigin};

/// Query parameters for symbol search.
///
/// Bundles all search options into a single struct to avoid long argument
/// lists in [`crate::cli::search::run`] and [`crate::index::reader::IndexReader::search`].
#[derive(Debug)]
pub struct SearchQuery<'a> {
    /// Symbol name, FQN, or regex pattern.
    pub query: &'a str,
    /// Filter by symbol type (`"any"`, `"class"`, `"method"`, `"field"`).
    pub symbol_type: &'a str,
    /// Exact FQN match mode.
    pub fqn_mode: bool,
    /// Regex search mode.
    pub regex_mode: bool,
    /// Maximum number of results.
    pub limit: usize,
    /// Number of results to skip (for pagination).
    pub offset: usize,
    /// Restrict to a specific dependency GAV.
    pub dependency: Option<&'a str>,
    /// Filter by visibility levels (`None` = all).
    pub access_levels: Option<&'a [&'a str]>,
}

/// Structured output for the `search` command.
#[derive(Debug, Serialize)]
pub struct SearchOutput {
    /// The original query string.
    pub query: String,
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
    /// Primary source view (source or decompiler).
    pub primary: SourceView,
    /// Optional secondary view (e.g. decompiler Java for Kotlin sources).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<SourceView>,
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
