//! Tantivy schema definition for the symbol index.
//!
//! # Field categories
//!
//! | Category | Fields | Tantivy options | Purpose |
//! |----------|--------|----------------|---------|
//! | **Identity** | `gav`, `symbol_kind`, `fqn`, `package`, `class_name`, `classpaths` | `STRING \| STORED` (`symbol_kind` also `FAST`) | Exact-match filtering |
//! | **Search** | `simple_name`, `name_parts`, `simple_name_rev`, `package_rev` | `TEXT \| STORED` / `TEXT` / `STRING` | Tokenized full-text search; `_rev` fields for suffix glob acceleration |
//! | **Metadata** | `descriptor`, `signature_java`, `signature_kotlin`, `access_flags`, `source`, `source_path`, `source_language`, `source_file_name` | `STORED` (some `STRING \| STORED`); `access_level` also `FAST`) | Carried in results, not directly searchable |

use tantivy::schema::*;

/// Build the Tantivy schema for the symbol index.
///
/// Fields are organized into three categories:
///
/// - **Identity fields** (untokenized, exact-match): `gav`, `symbol_kind`, `fqn`,
///   `package`, `class_name` -- used for filtering and precise lookups.
///   `symbol_kind` also carries `FAST` for columnar filtering.
/// - **Search fields** (tokenized): `simple_name`, `name_parts` -- used for
///   full-text search queries. `simple_name_rev` (TEXT) and `package_rev` (STRING)
///   hold reversed values for suffix glob acceleration.
/// - **Metadata fields** (stored only): `descriptor`, `signature_java`,
///   `signature_kotlin`, `access_flags`, `source`, `source_path` -- carried
///   in results but not directly searchable. `access_level` also carries `FAST`
///   for columnar filtering.
pub fn build_schema() -> Schema {
    let mut builder = Schema::builder();

    // Identity fields (untokenized, exact match)
    builder.add_text_field("gav", STRING | STORED);
    builder.add_text_field("symbol_kind", STRING | STORED | FAST);
    builder.add_text_field("fqn", STRING | STORED);
    builder.add_text_field("package", STRING | STORED);
    builder.add_text_field("class_name", STRING | STORED);

    // Search fields (tokenized)
    builder.add_text_field("simple_name", TEXT | STORED);
    builder.add_text_field("name_parts", TEXT);

    // Search fields — reversed (for suffix glob acceleration)
    builder.add_text_field("simple_name_rev", TEXT);
    builder.add_text_field("package_rev", STRING);

    // Metadata fields (stored only)
    builder.add_text_field("descriptor", STORED);
    builder.add_text_field("signature_java", STORED);
    builder.add_text_field("signature_kotlin", STORED);
    builder.add_text_field("access_flags", STRING | STORED);
    builder.add_text_field("access_level", STRING | STORED | FAST);
    builder.add_text_field("source", STRING | STORED);
    builder.add_text_field("source_path", STORED);
    builder.add_text_field("source_language", STRING | STORED);
    builder.add_text_field("source_file_name", STORED);
    builder.add_text_field("classpaths", STRING | STORED);

    builder.build()
}
