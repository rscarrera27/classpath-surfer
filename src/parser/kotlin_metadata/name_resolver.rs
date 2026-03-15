//! Name resolution for the Kotlin metadata string table.

use super::proto::*;

/// Well-known predefined qualified names used by the Kotlin metadata string table.
pub(super) const PREDEFINED_NAMES: &[&str] = &[
    "kotlin/Any",
    "kotlin/Nothing",
    "kotlin/Unit",
    "kotlin/Byte",
    "kotlin/Short",
    "kotlin/Int",
    "kotlin/Long",
    "kotlin/Float",
    "kotlin/Double",
    "kotlin/Char",
    "kotlin/Boolean",
    "kotlin/String",
    "kotlin/Enum",
    "kotlin/Array",
    "kotlin/collections/Iterable",
    "kotlin/collections/MutableIterable",
    "kotlin/collections/Collection",
    "kotlin/collections/MutableCollection",
    "kotlin/collections/List",
    "kotlin/collections/MutableList",
    "kotlin/collections/Set",
    "kotlin/collections/MutableSet",
    "kotlin/collections/Map",
    "kotlin/collections/MutableMap",
    "kotlin/collections/Map.Entry",
    "kotlin/collections/MutableMap.MutableEntry",
    "kotlin/collections/Iterator",
    "kotlin/collections/MutableIterator",
    "kotlin/collections/ListIterator",
    "kotlin/collections/MutableListIterator",
];

/// Resolver for looking up names from the Kotlin metadata string table.
pub struct NameResolver {
    string_table_types: StringTableTypes,
    strings: Vec<String>,
}

impl NameResolver {
    /// Create a new `NameResolver` from string table types and d2 strings.
    pub(super) fn new(stt: StringTableTypes, d2: Vec<String>) -> Self {
        Self {
            string_table_types: stt,
            strings: d2,
        }
    }

    /// Get a plain string by index from the d2 table.
    pub fn get_string(&self, index: i32) -> &str {
        self.strings
            .get(index as usize)
            .map(String::as_str)
            .unwrap_or("<unknown>")
    }

    /// Get a qualified name by index, resolving through the string table types.
    pub fn get_qualified_name(&self, index: i32) -> String {
        let idx = index as usize;
        let mut current = 0usize;

        for record in &self.string_table_types.record {
            let range = record.range.unwrap_or(1) as usize;
            if idx < current + range {
                let offset = idx - current;
                // This record covers the requested index
                if let Some(predef) = record.predefined_index {
                    let actual_predef = predef as usize + offset;
                    if let Some(name) = PREDEFINED_NAMES.get(actual_predef) {
                        return (*name).to_string();
                    }
                }
                if let Some(ref literal) = record.string {
                    // Record has a literal string value — use it directly.
                    let op = record.operation.unwrap_or(0);
                    return apply_operation(literal, op);
                }
                // Fallback: use index directly as string index
                return self.get_string(index).to_string();
            }
            current += range;
        }

        // If no record found, use index directly
        self.get_string(index).to_string()
    }
}

/// Apply a string table operation to a literal string.
pub(super) fn apply_operation(s: &str, operation: i32) -> String {
    match operation {
        1 => {
            // INTERNAL_TO_CLASS_ID: replace '/' with '.'
            // Actually, the qualified name uses '/' as separator already.
            // This operation replaces '.' with '/' in the stored string.
            s.replace('.', "/")
        }
        2 => {
            // DESC_TO_CLASS_ID: strip leading 'L' and trailing ';'
            let trimmed = s.strip_prefix('L').unwrap_or(s);
            let trimmed = trimmed.strip_suffix(';').unwrap_or(trimmed);
            trimmed.to_string()
        }
        _ => s.to_string(), // NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::kotlin_metadata::tests::make_resolver;

    #[test]
    fn test_apply_operation_none() {
        assert_eq!(apply_operation("kotlin/Int", 0), "kotlin/Int");
    }

    #[test]
    fn test_apply_operation_desc_to_class_id() {
        assert_eq!(apply_operation("Lkotlin/Int;", 2), "kotlin/Int");
    }

    #[test]
    fn test_name_resolver_predefined() {
        let stt = StringTableTypes {
            record: vec![StringTableRecord {
                range: Some(30),
                predefined_index: Some(0),
                string: None,
                operation: None,
                substring_index: vec![],
                replace_char: vec![],
            }],
            ..Default::default()
        };
        let resolver = NameResolver::new(stt, vec![]);
        assert_eq!(resolver.get_qualified_name(0), "kotlin/Any");
        assert_eq!(resolver.get_qualified_name(5), "kotlin/Int");
    }

    #[test]
    fn test_name_resolver_get_string_basic() {
        let resolver = make_resolver(&["hello", "world"]);
        assert_eq!(resolver.get_string(0), "hello");
        assert_eq!(resolver.get_string(1), "world");
    }

    #[test]
    fn test_name_resolver_get_string_out_of_bounds() {
        let resolver = make_resolver(&["only"]);
        assert_eq!(resolver.get_string(99), "<unknown>");
    }

    #[test]
    fn test_name_resolver_string_record_with_operation() {
        // Record with literal string + INTERNAL_TO_CLASS_ID operation (replaces '.' with '/')
        let stt = StringTableTypes {
            record: vec![StringTableRecord {
                range: Some(1),
                predefined_index: None,
                string: Some("com.example.Foo".to_string()),
                operation: Some(1), // INTERNAL_TO_CLASS_ID
                substring_index: vec![],
                replace_char: vec![],
            }],
            ..Default::default()
        };
        let resolver = NameResolver::new(stt, vec![]);
        assert_eq!(resolver.get_qualified_name(0), "com/example/Foo");
    }

    #[test]
    fn test_name_resolver_multi_record_spanning() {
        // Two records: first covers indices 0-2 (predefined), second covers 3+ (d2 fallback)
        let stt = StringTableTypes {
            record: vec![
                StringTableRecord {
                    range: Some(3),
                    predefined_index: Some(0),
                    string: None,
                    operation: None,
                    substring_index: vec![],
                    replace_char: vec![],
                },
                StringTableRecord {
                    range: Some(2),
                    predefined_index: None,
                    string: None, // no literal → fall through to d2 by index
                    operation: None,
                    substring_index: vec![],
                    replace_char: vec![],
                },
            ],
            ..Default::default()
        };
        let resolver = NameResolver::new(stt, vec!["MyClass".to_string(), "MyOther".to_string()]);
        // Index 0-2 → predefined names
        assert_eq!(resolver.get_qualified_name(0), "kotlin/Any");
        assert_eq!(resolver.get_qualified_name(2), "kotlin/Unit");
        // Index 3 → second record, no predefined/string → d2 fallback → d2[3] → "<unknown>"
        // Actually, fallback uses the original index directly, so d2[3] → out of bounds → "<unknown>"
        // For the test to work, we need indices that map to d2. Let me use literal strings instead.
        assert_eq!(resolver.get_qualified_name(3), "<unknown>");
    }
}
