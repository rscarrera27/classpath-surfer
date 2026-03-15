//! Tantivy schema definition for the symbol index.
//!
//! # Field categories
//!
//! | Category | Fields | Tantivy options | Purpose |
//! |----------|--------|----------------|---------|
//! | **Identity** | `gav`, `symbol_kind`, `fqn`, `package`, `class_name`, `scopes` | `STRING \| STORED` | Exact-match filtering |
//! | **Search** | `simple_name`, `name_parts` | `TEXT \| STORED` / `TEXT` | Tokenized full-text search |
//! | **Metadata** | `descriptor`, `signature_java`, `signature_kotlin`, `access_flags`, `source`, `source_path`, `source_language`, `source_file_name` | `STORED` (some `STRING \| STORED`) | Carried in results, not directly searchable |

use tantivy::schema::*;

/// Build the Tantivy schema for the symbol index.
///
/// Fields are organized into three categories:
///
/// - **Identity fields** (untokenized, exact-match): `gav`, `symbol_kind`, `fqn`,
///   `package`, `class_name` -- used for filtering and precise lookups.
/// - **Search fields** (tokenized): `simple_name`, `name_parts` -- used for
///   full-text search queries.
/// - **Metadata fields** (stored only): `descriptor`, `signature_java`,
///   `signature_kotlin`, `access_flags`, `source`, `source_path` -- carried
///   in results but not directly searchable.
pub fn build_schema() -> Schema {
    let mut builder = Schema::builder();

    // Identity fields (untokenized, exact match)
    builder.add_text_field("gav", STRING | STORED);
    builder.add_text_field("symbol_kind", STRING | STORED);
    builder.add_text_field("fqn", STRING | STORED);
    builder.add_text_field("package", STRING | STORED);
    builder.add_text_field("class_name", STRING | STORED);

    // Search fields (tokenized)
    builder.add_text_field("simple_name", TEXT | STORED);
    builder.add_text_field("name_parts", TEXT);

    // Metadata fields (stored only)
    builder.add_text_field("descriptor", STORED);
    builder.add_text_field("signature_java", STORED);
    builder.add_text_field("signature_kotlin", STORED);
    builder.add_text_field("access_flags", STRING | STORED);
    builder.add_text_field("access_level", STRING | STORED);
    builder.add_text_field("source", STRING | STORED);
    builder.add_text_field("source_path", STORED);
    builder.add_text_field("source_language", STRING | STORED);
    builder.add_text_field("source_file_name", STORED);
    builder.add_text_field("scopes", STRING | STORED);

    builder.build()
}
