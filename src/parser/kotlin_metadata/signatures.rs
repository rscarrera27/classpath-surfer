//! JVM descriptor hints and signature map building.

use std::collections::HashMap;

use super::KotlinSignatures;
use super::name_resolver::NameResolver;
use super::proto::*;

/// Build an approximate JVM descriptor hint for matching functions.
/// This is not a full descriptor — just enough info to disambiguate overloads.
pub(super) fn build_jvm_descriptor_hint(
    func: &Function,
    _resolver: &NameResolver,
    _type_table: Option<&TypeTable>,
) -> String {
    // Count parameters for basic disambiguation
    let param_count = func.value_parameter.len();
    let flags = func.flags.or(func.old_flags).unwrap_or(6);
    let is_suspend = flags & (1 << 14) != 0;
    let total = if is_suspend {
        param_count + 1
    } else {
        param_count
    };
    format!("params:{total}")
}

/// Generate the JVM getter method name for a Kotlin property.
pub(super) fn property_getter_name(property_name: &str) -> String {
    let mut chars = property_name.chars();
    match chars.next() {
        Some(first) => {
            let capitalized: String = first.to_uppercase().chain(chars).collect();
            format!("get{capitalized}")
        }
        None => "get".to_string(),
    }
}

/// Build a lookup map from JVM method name to Kotlin signature.
///
/// For overloaded methods, the first match wins (best-effort).
pub fn build_signature_map(sigs: &KotlinSignatures) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for member in &sigs.members {
        // Use method name as key; first match wins for overloads
        map.entry(member.jvm_name.clone())
            .or_insert_with(|| member.kotlin_display.clone());
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::kotlin_metadata::KotlinMemberSignature;

    #[test]
    fn test_property_getter_name() {
        assert_eq!(property_getter_name("name"), "getName");
        assert_eq!(property_getter_name("count"), "getCount");
        assert_eq!(property_getter_name("x"), "getX");
    }

    #[test]
    fn test_build_signature_map() {
        let sigs = KotlinSignatures {
            class_display: Some("class Foo".to_string()),
            members: vec![
                KotlinMemberSignature {
                    jvm_name: "bar".to_string(),
                    jvm_descriptor: "params:0".to_string(),
                    kotlin_display: "fun bar()".to_string(),
                },
                KotlinMemberSignature {
                    jvm_name: "bar".to_string(),
                    jvm_descriptor: "params:1".to_string(),
                    kotlin_display: "fun bar(x: Int)".to_string(),
                },
                KotlinMemberSignature {
                    jvm_name: "baz".to_string(),
                    jvm_descriptor: "params:0".to_string(),
                    kotlin_display: "fun baz()".to_string(),
                },
            ],
        };
        let map = build_signature_map(&sigs);
        // first-wins for overloaded "bar"
        assert_eq!(map.get("bar").unwrap(), "fun bar()");
        assert_eq!(map.get("baz").unwrap(), "fun baz()");
        assert_eq!(map.len(), 2);
    }
}
