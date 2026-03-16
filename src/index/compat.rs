//! Index schema compatibility checking.

/// Required field names for schema compatibility validation.
///
/// When new fields are added to the schema, append them here.
/// Both `open_or_create_index` and `IndexReader::open` use this list
/// to detect outdated indexes that need rebuilding.
pub const REQUIRED_FIELDS: &[&str] = &[
    "gav",
    "symbol_kind",
    "fqn",
    "simple_name",
    "name_parts",
    "signature_java",
    "signature_kotlin",
    "access_flags",
    "access_level",
    "source",
    "source_language",
    "classpaths",
    "simple_name_rev",
    "package_rev",
];
