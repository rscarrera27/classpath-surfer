//! `.class` file parsing and symbol extraction.
//!
//! Uses the `cafebabe` crate to parse JVM classfiles and extract class, method,
//! and field symbols as [`SymbolDoc`](crate::model::SymbolDoc) values.
//!
//! # Key data structures
//!
//! This module produces `Vec<SymbolDoc>` from raw class bytes.  Each symbol
//! carries the SourceFile attribute value (`source_file_name`) which, combined
//! with the `package`, forms the deterministic join key for source-JAR matching
//! (see `parser::jar::SourceTable`).
//!
//! For Kotlin classes the module also extracts `@kotlin.Metadata` annotations
//! and decodes them into Kotlin-style signatures via the `kotlin_metadata`
//! submodule.

use anyhow::Result;
use cafebabe::{
    ClassAccessFlags, ClassFile, FieldAccessFlags, FieldInfo, MethodAccessFlags, MethodInfo,
    attributes::{AnnotationElementValue, AttributeData},
    parse_class,
};

use crate::model::{SignatureDisplay, SourceLanguage, SourceOrigin, SymbolDoc, SymbolKind};

use super::descriptor;
use super::kotlin_metadata::{self, KotlinMetadataRaw};

/// Extract symbols (class, methods, fields) from a .class file's bytes.
pub fn extract_symbols(class_bytes: &[u8], gav: &str) -> Result<Vec<SymbolDoc>> {
    let class =
        parse_class(class_bytes).map_err(|e| anyhow::anyhow!("failed to parse class: {e}"))?;

    let class_name_raw: &str = class.this_class.as_ref(); // "com/google/common/collect/ImmutableList"
    let fqn = class_name_raw.replace('/', ".");

    // Skip module-info, package-info
    if fqn.ends_with(".module-info") || fqn.ends_with(".package-info") {
        return Ok(vec![]);
    }

    // Skip anonymous classes (Foo$1, Foo$2, etc.)
    let simple_class = simple_name_from_fqn(&fqn);
    if is_anonymous_class(&simple_class) {
        return Ok(vec![]);
    }

    // Skip synthetic classes
    if class.access_flags.contains(ClassAccessFlags::SYNTHETIC) {
        return Ok(vec![]);
    }

    let package = package_from_fqn(&fqn);
    let class_access = format_class_access(&class);
    let class_access_level = class_access_level(&class);
    let name_parts = split_camel_case(&simple_class);

    // A1: Extract SourceFile attribute for source language detection
    let source_file_name = extract_source_file_name(&class);
    let source_language = source_file_name.as_ref().map(|n| {
        if n.ends_with(".kt") {
            SourceLanguage::Kotlin
        } else if n.ends_with(".java") {
            SourceLanguage::Java
        } else if n.ends_with(".scala") {
            SourceLanguage::Scala
        } else if n.ends_with(".groovy") {
            SourceLanguage::Groovy
        } else if n.ends_with(".clj") {
            SourceLanguage::Clojure
        } else {
            SourceLanguage::Unknown
        }
    });

    // B1: Extract Kotlin metadata and generate Kotlin signatures
    let kotlin_sigs = if source_language == Some(SourceLanguage::Kotlin) {
        find_kotlin_metadata(&class)
            .and_then(|raw| kotlin_metadata::extract_kotlin_signatures(&raw))
    } else {
        None
    };
    let kt_sig_map = kotlin_sigs
        .as_ref()
        .map(kotlin_metadata::build_signature_map)
        .unwrap_or_default();

    let mut symbols = Vec::new();

    // Class symbol
    let kotlin_class_display = kotlin_sigs.as_ref().and_then(|s| s.class_display.clone());
    symbols.push(SymbolDoc {
        gav: gav.to_string(),
        symbol_kind: SymbolKind::Class,
        fqn: fqn.clone(),
        package: package.clone(),
        class_name: simple_class.clone(),
        simple_name: simple_class.clone(),
        name_parts: name_parts.clone(),
        descriptor: String::new(),
        signature: SignatureDisplay {
            java: format_class_display(&class, &class_access, &simple_class),
            kotlin: kotlin_class_display,
        },
        access_flags: class_access,
        access_level: class_access_level.to_string(),
        source: SourceOrigin::Decompiled {
            source_language,
            source_file_name: source_file_name.clone(),
        },
    });

    // Method symbols
    for method in &class.methods {
        if should_skip_method(method) {
            continue;
        }

        let method_name = method.name.as_ref();
        let is_constructor = method_name == "<init>";
        let display_name = if is_constructor {
            simple_class.clone()
        } else {
            method_name.to_string()
        };

        let method_fqn = format!("{fqn}.{display_name}");
        let access = format_method_access(method);
        let m_access_level = method_access_level(method);
        let desc = method.descriptor.to_string();

        // Look up Kotlin signature by JVM method name
        let kotlin_sig = kt_sig_map.get(method_name).cloned();

        symbols.push(SymbolDoc {
            gav: gav.to_string(),
            symbol_kind: SymbolKind::Method,
            fqn: method_fqn,
            package: package.clone(),
            class_name: simple_class.clone(),
            simple_name: display_name.clone(),
            name_parts: split_camel_case(&display_name),
            descriptor: desc.to_string(),
            signature: SignatureDisplay {
                java: descriptor::format_method_display(&access, &simple_class, method_name, &desc),
                kotlin: kotlin_sig,
            },
            access_flags: access,
            access_level: m_access_level.to_string(),
            source: SourceOrigin::Decompiled {
                source_language,
                source_file_name: source_file_name.clone(),
            },
        });
    }

    // Field symbols
    for field in &class.fields {
        if should_skip_field(field) {
            continue;
        }

        let field_name = field.name.as_ref().to_string();
        let field_fqn = format!("{fqn}.{field_name}");
        let access = format_field_access(field);
        let f_access_level = field_access_level(field);
        let desc = field.descriptor.to_string();

        // Look up Kotlin signature (property mapped to field)
        let kotlin_sig = kt_sig_map.get(&field_name).cloned();

        symbols.push(SymbolDoc {
            gav: gav.to_string(),
            symbol_kind: SymbolKind::Field,
            fqn: field_fqn,
            package: package.clone(),
            class_name: simple_class.clone(),
            simple_name: field_name.clone(),
            name_parts: split_camel_case(&field_name),
            descriptor: desc.to_string(),
            signature: SignatureDisplay {
                java: descriptor::format_field_display(&access, &field_name, &desc),
                kotlin: kotlin_sig,
            },
            access_flags: access,
            access_level: f_access_level.to_string(),
            source: SourceOrigin::Decompiled {
                source_language,
                source_file_name: source_file_name.clone(),
            },
        });
    }

    Ok(symbols)
}

/// Extract the SourceFile attribute value from a class.
pub fn extract_source_file_name(class: &ClassFile) -> Option<String> {
    class.attributes.iter().find_map(|attr| {
        if let AttributeData::SourceFile(name) = &attr.data {
            Some(name.to_string())
        } else {
            None
        }
    })
}

/// Extract the SourceFile attribute from raw class bytes.
pub fn source_file_name_from_bytes(class_bytes: &[u8]) -> Option<String> {
    let class = parse_class(class_bytes).ok()?;
    extract_source_file_name(&class)
}

/// Find and extract `@kotlin.Metadata` annotation fields from a class.
fn find_kotlin_metadata(class: &ClassFile) -> Option<KotlinMetadataRaw> {
    for attr in &class.attributes {
        if let AttributeData::RuntimeVisibleAnnotations(annotations) = &attr.data {
            for ann in annotations {
                if ann.type_descriptor.to_string() == "Lkotlin/Metadata;" {
                    return extract_metadata_fields(ann);
                }
            }
        }
    }
    None
}

/// Extract k, d1, d2 fields from a Kotlin Metadata annotation.
fn extract_metadata_fields(ann: &cafebabe::attributes::Annotation) -> Option<KotlinMetadataRaw> {
    let mut k: Option<i32> = None;
    let mut d1: Vec<String> = Vec::new();
    let mut d2: Vec<String> = Vec::new();

    for element in &ann.elements {
        match element.name.as_ref() {
            "k" => {
                if let AnnotationElementValue::IntConstant(v) = &element.value {
                    k = Some(*v);
                }
            }
            "d1" => {
                if let AnnotationElementValue::ArrayValue(arr) = &element.value {
                    for item in arr {
                        if let AnnotationElementValue::StringConstant(s) = item {
                            d1.push(s.to_string());
                        }
                    }
                }
            }
            "d2" => {
                if let AnnotationElementValue::ArrayValue(arr) = &element.value {
                    for item in arr {
                        if let AnnotationElementValue::StringConstant(s) = item {
                            d2.push(s.to_string());
                        }
                    }
                }
            }
            _ => {} // mv, bv, xi, xs — not needed
        }
    }

    Some(KotlinMetadataRaw {
        k: k.unwrap_or(1),
        d1,
        d2,
    })
}

fn should_skip_method(method: &MethodInfo) -> bool {
    let name = method.name.as_ref();
    // Skip static initializers
    if name == "<clinit>" {
        return true;
    }
    // Skip synthetic/bridge methods
    if method.access_flags.contains(MethodAccessFlags::SYNTHETIC)
        || method.access_flags.contains(MethodAccessFlags::BRIDGE)
    {
        return true;
    }
    false
}

fn should_skip_field(field: &FieldInfo) -> bool {
    // Skip synthetic fields
    field.access_flags.contains(FieldAccessFlags::SYNTHETIC)
}

fn is_anonymous_class(simple_name: &str) -> bool {
    // "Foo$1" or just "1" — anonymous/numbered inner class
    simple_name
        .rsplit('$')
        .next()
        .is_some_and(|last| last.chars().all(|c| c.is_ascii_digit()))
}

fn simple_name_from_fqn(fqn: &str) -> String {
    fqn.rsplit('.').next().unwrap_or(fqn).replace('$', ".")
}

/// Extract the package name from a fully qualified class name.
///
/// # Examples
///
/// ```
/// use classpath_surfer::parser::classfile::package_from_fqn;
///
/// assert_eq!(package_from_fqn("com.google.common.collect.ImmutableList"), "com.google.common.collect");
/// assert_eq!(package_from_fqn("Foo"), "");
/// ```
pub fn package_from_fqn(fqn: &str) -> String {
    match fqn.rfind('.') {
        Some(i) => fqn[..i].to_string(),
        None => String::new(),
    }
}

fn format_class_access(class: &ClassFile) -> String {
    let mut parts = Vec::new();
    let flags = class.access_flags;
    if flags.contains(ClassAccessFlags::PUBLIC) {
        parts.push("public");
    }
    if flags.contains(ClassAccessFlags::ABSTRACT) && !flags.contains(ClassAccessFlags::INTERFACE) {
        parts.push("abstract");
    }
    if flags.contains(ClassAccessFlags::FINAL) {
        parts.push("final");
    }
    parts.join(" ")
}

fn class_access_level(class: &ClassFile) -> &'static str {
    if class.access_flags.contains(ClassAccessFlags::PUBLIC) {
        "public"
    } else {
        "package_private"
    }
}

fn method_access_level(method: &MethodInfo) -> &'static str {
    let flags = method.access_flags;
    if flags.contains(MethodAccessFlags::PUBLIC) {
        "public"
    } else if flags.contains(MethodAccessFlags::PROTECTED) {
        "protected"
    } else if flags.contains(MethodAccessFlags::PRIVATE) {
        "private"
    } else {
        "package_private"
    }
}

fn field_access_level(field: &FieldInfo) -> &'static str {
    let flags = field.access_flags;
    if flags.contains(FieldAccessFlags::PUBLIC) {
        "public"
    } else if flags.contains(FieldAccessFlags::PROTECTED) {
        "protected"
    } else if flags.contains(FieldAccessFlags::PRIVATE) {
        "private"
    } else {
        "package_private"
    }
}

fn format_class_display(class: &ClassFile, access: &str, simple_name: &str) -> String {
    let kind = if class.access_flags.contains(ClassAccessFlags::INTERFACE) {
        "interface"
    } else if class.access_flags.contains(ClassAccessFlags::ENUM) {
        "enum"
    } else if class.access_flags.contains(ClassAccessFlags::ANNOTATION) {
        "@interface"
    } else {
        "class"
    };

    if access.is_empty() {
        format!("{kind} {simple_name}")
    } else {
        format!("{access} {kind} {simple_name}")
    }
}

fn format_method_access(method: &MethodInfo) -> String {
    let mut parts = Vec::new();
    let flags = method.access_flags;
    if flags.contains(MethodAccessFlags::PUBLIC) {
        parts.push("public");
    } else if flags.contains(MethodAccessFlags::PROTECTED) {
        parts.push("protected");
    } else if flags.contains(MethodAccessFlags::PRIVATE) {
        parts.push("private");
    }
    if flags.contains(MethodAccessFlags::STATIC) {
        parts.push("static");
    }
    if flags.contains(MethodAccessFlags::FINAL) {
        parts.push("final");
    }
    if flags.contains(MethodAccessFlags::ABSTRACT) {
        parts.push("abstract");
    }
    if flags.contains(MethodAccessFlags::NATIVE) {
        parts.push("native");
    }
    if flags.contains(MethodAccessFlags::SYNCHRONIZED) {
        parts.push("synchronized");
    }
    parts.join(" ")
}

fn format_field_access(field: &FieldInfo) -> String {
    let mut parts = Vec::new();
    let flags = field.access_flags;
    if flags.contains(FieldAccessFlags::PUBLIC) {
        parts.push("public");
    } else if flags.contains(FieldAccessFlags::PROTECTED) {
        parts.push("protected");
    } else if flags.contains(FieldAccessFlags::PRIVATE) {
        parts.push("private");
    }
    if flags.contains(FieldAccessFlags::STATIC) {
        parts.push("static");
    }
    if flags.contains(FieldAccessFlags::FINAL) {
        parts.push("final");
    }
    if flags.contains(FieldAccessFlags::VOLATILE) {
        parts.push("volatile");
    }
    if flags.contains(FieldAccessFlags::TRANSIENT) {
        parts.push("transient");
    }
    parts.join(" ")
}

/// Split a CamelCase identifier into space-separated words.
///
/// # Examples
///
/// ```
/// use classpath_surfer::parser::classfile::split_camel_case;
///
/// assert_eq!(split_camel_case("ImmutableList"), "Immutable List");
/// assert_eq!(split_camel_case("parseJSON"), "parse JSON");
/// ```
pub fn split_camel_case(name: &str) -> String {
    let mut words = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = name.chars().collect();
    for i in 0..chars.len() {
        let c = chars[i];
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            // Split before uppercase if previous was lowercase,
            // or if previous was uppercase and next is lowercase (e.g., "parseJSON" → "parse JSON")
            if (prev.is_lowercase() || (prev.is_uppercase() && next_is_lower))
                && !current.is_empty()
            {
                words.push(current);
                current = String::new();
            }
        }
        current.push(c);
    }
    if !current.is_empty() {
        words.push(current);
    }

    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_camel_case() {
        assert_eq!(split_camel_case("ImmutableList"), "Immutable List");
        assert_eq!(split_camel_case("parseJSON"), "parse JSON");
        assert_eq!(split_camel_case("XMLParser"), "XML Parser");
        assert_eq!(split_camel_case("simple"), "simple");
        assert_eq!(split_camel_case("A"), "A");
        assert_eq!(split_camel_case("getHTTPSUrl"), "get HTTPS Url");
    }

    #[test]
    fn test_simple_name_from_fqn() {
        assert_eq!(
            simple_name_from_fqn("com.google.common.collect.ImmutableList"),
            "ImmutableList"
        );
        assert_eq!(simple_name_from_fqn("java.util.Map$Entry"), "Map.Entry");
    }

    #[test]
    fn test_package_from_fqn() {
        assert_eq!(
            package_from_fqn("com.google.common.collect.ImmutableList"),
            "com.google.common.collect"
        );
        assert_eq!(package_from_fqn("Foo"), "");
    }

    #[test]
    fn test_is_anonymous() {
        assert!(is_anonymous_class("Foo$1"));
        assert!(is_anonymous_class("Foo$123"));
        assert!(!is_anonymous_class("Foo$Bar"));
        assert!(!is_anonymous_class("Foo"));
    }
}
