//! Tantivy-based symbol index.
//!
//! Manages the full-text search index that stores symbols extracted from
//! library JARs. Includes schema definition, index writer, and reader.

/// Schema compatibility constants shared between reader and writer.
pub mod compat;
/// Index querying and search result construction.
pub mod reader;
/// Tantivy schema definition (identity, search, and metadata fields).
pub mod schema;
/// Index creation, document insertion, and GAV-level deletion.
pub mod writer;
