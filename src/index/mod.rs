//! Tantivy-based symbol index.
//!
//! Manages the full-text search index that stores symbols extracted from
//! dependency JARs. Includes schema definition, index writer, and reader.

/// Index querying and search result construction.
pub mod reader;
/// Tantivy schema definition (identity, search, and metadata fields).
pub mod schema;
/// Index creation, document insertion, and GAV-level deletion.
pub mod writer;
