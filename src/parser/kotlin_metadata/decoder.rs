//! Binary decoding and signature extraction from Kotlin metadata.

use prost::Message;

use super::format;
use super::name_resolver::NameResolver;
use super::proto::*;
use super::signatures::{build_jvm_descriptor_hint, property_getter_name};
use super::{KotlinMemberSignature, KotlinMetadataRaw, KotlinSignatures};

/// UTF-8 mode marker prepended by the Kotlin compiler's `BitEncoding.encodeBytes`.
///
/// When present as the first character of `d1[0]`, it indicates that the rest of
/// the data uses simple char-to-byte mapping (as opposed to the legacy 8-to-7 encoding).
const UTF8_MODE_MARKER: char = '\0';

/// Decode raw bytes from d1 strings.
///
/// The Kotlin compiler encodes protobuf data into annotation string arrays.
/// In UTF-8 mode (marker = `\0` at position 0), each subsequent character's
/// code point (0–255) maps directly to one byte.
pub(super) fn d1_to_bytes(d1: &[String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut first = true;
    for s in d1 {
        for c in s.chars() {
            // Strip the UTF-8 mode marker at the very start of d1[0].
            if first {
                first = false;
                if c == UTF8_MODE_MARKER {
                    continue;
                }
            }
            bytes.push(c as u8);
        }
    }
    bytes
}

/// Decode a varint from raw bytes, returning (value, bytes_consumed).
pub(super) fn decode_varint(buf: &[u8]) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    for (i, &byte) in buf.iter().enumerate() {
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

/// Extract Kotlin signatures from a classfile's `@kotlin.Metadata` annotation.
///
/// Returns `None` if the class has no Kotlin metadata or if decoding fails.
pub fn extract_kotlin_signatures(raw: &KotlinMetadataRaw) -> Option<KotlinSignatures> {
    let d1_bytes = d1_to_bytes(&raw.d1);
    if d1_bytes.is_empty() {
        return None;
    }

    match raw.k {
        1 => decode_class_signatures(&d1_bytes, &raw.d2),
        2 | 5 => decode_package_signatures(&d1_bytes, &raw.d2),
        _ => None, // synthetic (3), multi-file facade (4) — skip
    }
}

/// Decode signatures from a Kotlin class (k=1).
pub(super) fn decode_class_signatures(d1_bytes: &[u8], d2: &[String]) -> Option<KotlinSignatures> {
    // d1 format: length-delimited StringTableTypes, then Class message
    let (stt_len, varint_size) = decode_varint(d1_bytes)?;
    let stt_end = varint_size + stt_len as usize;
    if stt_end > d1_bytes.len() {
        return None;
    }

    let stt = StringTableTypes::decode(&d1_bytes[varint_size..stt_end]).ok()?;
    let class_proto = Class::decode(&d1_bytes[stt_end..]).ok()?;

    let resolver = NameResolver::new(stt, d2.to_vec());
    let type_table = class_proto.type_table.as_ref();

    // Class display
    let class_flags = class_proto.flags.unwrap_or(6);
    let class_name = class_proto
        .fq_name
        .map(|idx| {
            let qn = resolver.get_qualified_name(idx);
            format::kotlin_simple_name(&qn)
        })
        .unwrap_or_default();
    let class_display = Some(format::format_class_display(class_flags, &class_name));

    let mut members = Vec::new();

    // Functions
    for func in &class_proto.function {
        let name = resolver.get_string(func.name).to_string();
        let kotlin_display = format::format_function_display(
            func,
            &resolver,
            type_table,
            &class_proto.type_parameter,
        );

        // Build approximate JVM descriptor for matching
        let jvm_descriptor = build_jvm_descriptor_hint(func, &resolver, type_table);

        members.push(KotlinMemberSignature {
            jvm_name: name,
            jvm_descriptor,
            kotlin_display,
        });
    }

    // Properties → getter methods
    for prop in &class_proto.property {
        let name = resolver.get_string(prop.name).to_string();
        let kotlin_display = format::format_property_display(
            prop,
            &resolver,
            type_table,
            &class_proto.type_parameter,
        );

        // Getter JVM name: getName or isName (for boolean)
        let getter_name = property_getter_name(&name);
        members.push(KotlinMemberSignature {
            jvm_name: getter_name,
            jvm_descriptor: String::new(),
            kotlin_display: kotlin_display.clone(),
        });

        // Also map the property name directly (for field access)
        members.push(KotlinMemberSignature {
            jvm_name: name,
            jvm_descriptor: String::new(),
            kotlin_display,
        });
    }

    // Constructors
    for ctor in &class_proto.constructor {
        let kotlin_display = format::format_constructor_display(
            ctor,
            &resolver,
            type_table,
            &class_proto.type_parameter,
        );
        members.push(KotlinMemberSignature {
            jvm_name: "<init>".to_string(),
            jvm_descriptor: String::new(),
            kotlin_display,
        });
    }

    Some(KotlinSignatures {
        class_display,
        members,
    })
}

/// Decode signatures from a Kotlin package facade (k=2) or multi-file part (k=5).
pub(super) fn decode_package_signatures(
    d1_bytes: &[u8],
    d2: &[String],
) -> Option<KotlinSignatures> {
    let (stt_len, varint_size) = decode_varint(d1_bytes)?;
    let stt_end = varint_size + stt_len as usize;
    if stt_end > d1_bytes.len() {
        return None;
    }

    let stt = StringTableTypes::decode(&d1_bytes[varint_size..stt_end]).ok()?;
    let pkg = Package::decode(&d1_bytes[stt_end..]).ok()?;

    let resolver = NameResolver::new(stt, d2.to_vec());
    let type_table = pkg.type_table.as_ref();
    let empty_type_params = Vec::new();

    let mut members = Vec::new();

    for func in &pkg.function {
        let name = resolver.get_string(func.name).to_string();
        let kotlin_display =
            format::format_function_display(func, &resolver, type_table, &empty_type_params);
        let jvm_descriptor = build_jvm_descriptor_hint(func, &resolver, type_table);

        members.push(KotlinMemberSignature {
            jvm_name: name,
            jvm_descriptor,
            kotlin_display,
        });
    }

    for prop in &pkg.property {
        let name = resolver.get_string(prop.name).to_string();
        let kotlin_display =
            format::format_property_display(prop, &resolver, type_table, &empty_type_params);

        let getter_name = property_getter_name(&name);
        members.push(KotlinMemberSignature {
            jvm_name: getter_name,
            jvm_descriptor: String::new(),
            kotlin_display: kotlin_display.clone(),
        });

        members.push(KotlinMemberSignature {
            jvm_name: name,
            jvm_descriptor: String::new(),
            kotlin_display,
        });
    }

    Some(KotlinSignatures {
        class_display: None,
        members,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::kotlin_metadata::tests::{class_type, encode_d1};

    #[test]
    fn test_d1_to_bytes() {
        let d1 = vec!["ABC".to_string()];
        assert_eq!(d1_to_bytes(&d1), vec![65, 66, 67]);
    }

    #[test]
    fn test_d1_to_bytes_multiple() {
        let d1 = vec!["AB".to_string(), "CD".to_string()];
        assert_eq!(d1_to_bytes(&d1), vec![65, 66, 67, 68]);
    }

    #[test]
    fn test_decode_varint() {
        assert_eq!(decode_varint(&[0x00]), Some((0, 1)));
        assert_eq!(decode_varint(&[0x01]), Some((1, 1)));
        assert_eq!(decode_varint(&[0x80, 0x01]), Some((128, 2)));
        assert_eq!(decode_varint(&[0xAC, 0x02]), Some((300, 2)));
    }

    #[test]
    fn test_extract_signatures_class_e2e() {
        // Build a class with one function and one property, encode to protobuf, decode
        // d2: [0]=MyClass, [1]=greet, [2]=kotlin/Unit, [3]=count, [4]=kotlin/Int
        let d2: Vec<String> = vec!["MyClass", "greet", "kotlin/Unit", "count", "kotlin/Int"]
            .into_iter()
            .map(String::from)
            .collect();

        let stt = StringTableTypes {
            record: vec![],
            ..Default::default()
        };

        let class = Class {
            flags: Some(6), // public final class
            fq_name: Some(0),
            type_parameter: vec![],
            supertype: vec![],
            supertype_id: vec![],
            constructor: vec![],
            function: vec![Function {
                flags: Some(6),
                old_flags: None,
                name: 1,                          // "greet"
                return_type: Some(class_type(2)), // Unit
                return_type_id: None,
                type_parameter: vec![],
                value_parameter: vec![],
                receiver_type: None,
                receiver_type_id: None,
                ..Default::default()
            }],
            property: vec![Property {
                flags: Some(6), // val
                old_flags: None,
                name: 3,                          // "count"
                return_type: Some(class_type(4)), // Int
                return_type_id: None,
                receiver_type: None,
                receiver_type_id: None,
                ..Default::default()
            }],
            type_table: None,
            ..Default::default()
        };

        let d1 = encode_d1(&stt, &class);
        let raw = KotlinMetadataRaw { k: 1, d1, d2 };
        let result = extract_kotlin_signatures(&raw).expect("should decode class");

        assert_eq!(result.class_display.as_deref(), Some("class MyClass"));

        let displays: Vec<&str> = result
            .members
            .iter()
            .map(|m| m.kotlin_display.as_str())
            .collect();
        assert!(
            displays.contains(&"fun greet()"),
            "should contain function: {displays:?}"
        );
        assert!(
            displays.contains(&"val count: Int"),
            "should contain property: {displays:?}"
        );
    }

    #[test]
    fn test_extract_signatures_package_e2e() {
        // Package (k=2) with a top-level function
        // d2: [0]=doStuff, [1]=kotlin/Unit
        let d2: Vec<String> = vec!["doStuff", "kotlin/Unit"]
            .into_iter()
            .map(String::from)
            .collect();

        let stt = StringTableTypes {
            record: vec![],
            ..Default::default()
        };
        let pkg = Package {
            function: vec![Function {
                flags: Some(6),
                old_flags: None,
                name: 0,
                return_type: Some(class_type(1)),
                return_type_id: None,
                type_parameter: vec![],
                value_parameter: vec![],
                receiver_type: None,
                receiver_type_id: None,
                ..Default::default()
            }],
            property: vec![],
            type_table: None,
            ..Default::default()
        };

        let d1 = encode_d1(&stt, &pkg);
        let raw = KotlinMetadataRaw { k: 2, d1, d2 };
        let result = extract_kotlin_signatures(&raw).expect("should decode package");

        assert!(
            result.class_display.is_none(),
            "package should have no class_display"
        );
        assert_eq!(result.members.len(), 1);
        assert_eq!(result.members[0].kotlin_display, "fun doStuff()");
    }

    #[test]
    fn test_extract_signatures_unknown_kind() {
        let raw = KotlinMetadataRaw {
            k: 3, // synthetic class
            d1: vec!["x".to_string()],
            d2: vec![],
        };
        assert!(extract_kotlin_signatures(&raw).is_none());
    }

    #[test]
    fn test_extract_signatures_empty_d1() {
        let raw = KotlinMetadataRaw {
            k: 1,
            d1: vec![],
            d2: vec![],
        };
        assert!(extract_kotlin_signatures(&raw).is_none());
    }

    /// Diagnostic test: load real CoroutineScope.class from a local Gradle cache JAR.
    /// Requires kotlinx-coroutines-core-jvm-1.9.0.jar to be present.
    #[test]
    #[ignore]
    fn diag_real_coroutinescope() {
        use cafebabe::attributes::{AnnotationElementValue, AttributeData};
        use std::io::Read as _;

        let jar_path = std::path::Path::new(env!("HOME"))
            .join(".gradle/caches/modules-2/files-2.1/org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm/1.9.0/9beade4c1c1569e4f36cbd2c37e02e3e41502601/kotlinx-coroutines-core-jvm-1.9.0.jar");
        if !jar_path.exists() {
            eprintln!("JAR not found, skipping diagnostic");
            return;
        }

        let file = std::fs::File::open(&jar_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut class_bytes = Vec::new();
        {
            let mut entry = archive
                .by_name("kotlinx/coroutines/CoroutineScope.class")
                .unwrap();
            entry.read_to_end(&mut class_bytes).unwrap();
        }
        eprintln!("class_bytes len = {}", class_bytes.len());

        // Step 1: parse with cafebabe
        let class = cafebabe::parse_class(&class_bytes)
            .expect("cafebabe should parse CoroutineScope.class");
        eprintln!("this_class = {}", class.this_class);

        // Step 2: find kotlin metadata annotation (inline extraction)
        let mut raw: Option<KotlinMetadataRaw> = None;
        for attr in &class.attributes {
            if let AttributeData::RuntimeVisibleAnnotations(annotations) = &attr.data {
                for ann in annotations {
                    if ann.type_descriptor.to_string() == "Lkotlin/Metadata;" {
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
                                    if let AnnotationElementValue::ArrayValue(arr) = &element.value
                                    {
                                        for item in arr {
                                            if let AnnotationElementValue::StringConstant(s) = item
                                            {
                                                d1.push(s.to_string());
                                            }
                                        }
                                    }
                                }
                                "d2" => {
                                    if let AnnotationElementValue::ArrayValue(arr) = &element.value
                                    {
                                        for item in arr {
                                            if let AnnotationElementValue::StringConstant(s) = item
                                            {
                                                d2.push(s.to_string());
                                            }
                                        }
                                    }
                                }
                                other => eprintln!("  annotation field: {other}"),
                            }
                        }
                        raw = Some(KotlinMetadataRaw {
                            k: k.unwrap_or(1),
                            d1,
                            d2,
                        });
                    }
                }
            }
        }
        eprintln!("kotlin metadata found = {}", raw.is_some());
        let raw = raw.expect("CoroutineScope should have @kotlin.Metadata");
        eprintln!(
            "k={}, d1.len()={}, d2.len()={}",
            raw.k,
            raw.d1.len(),
            raw.d2.len()
        );
        if !raw.d2.is_empty() {
            eprintln!("d2[..10] = {:?}", &raw.d2[..raw.d2.len().min(10)]);
        }

        // Step 3: inspect d1 chars vs bytes
        let d1_str = &raw.d1[0];
        eprintln!("d1 string char count = {}", d1_str.chars().count());
        eprintln!("d1 string byte len = {}", d1_str.len());
        let has_high_chars = d1_str.chars().any(|c| c as u32 > 255);
        eprintln!("has chars > U+00FF = {has_high_chars}");
        if has_high_chars {
            for (i, c) in d1_str.chars().enumerate().take(60) {
                if c as u32 > 255 {
                    eprintln!(
                        "  char[{i}] = U+{:04X} (as u8 = 0x{:02X})",
                        c as u32, c as u8
                    );
                }
            }
        }

        let d1_bytes = d1_to_bytes(&raw.d1);
        eprintln!("d1_bytes len = {}", d1_bytes.len());
        if !d1_bytes.is_empty() {
            eprintln!(
                "d1_bytes first 30 = {:02x?}",
                &d1_bytes[..d1_bytes.len().min(30)]
            );
        }

        // Step 4: decode varint
        let varint_result = decode_varint(&d1_bytes);
        eprintln!("decode_varint = {:?}", varint_result);
        let (stt_len, varint_size) = varint_result.expect("varint should decode");
        let stt_end = varint_size + stt_len as usize;
        eprintln!(
            "stt_len={stt_len}, varint_size={varint_size}, stt_end={stt_end}, total={}",
            d1_bytes.len()
        );

        // Step 5: decode STT
        let stt_result = StringTableTypes::decode(&d1_bytes[varint_size..stt_end]);
        eprintln!("STT decode = {:?}", stt_result.is_ok());
        if let Err(ref e) = stt_result {
            eprintln!("STT decode ERROR: {e}");
        }
        let stt = stt_result.expect("STT should decode");
        eprintln!("STT records count = {}", stt.record.len());
        for (i, r) in stt.record.iter().enumerate() {
            eprintln!(
                "  record[{i}]: range={:?} predef={:?} string={:?} op={:?}",
                r.range, r.predefined_index, r.string, r.operation
            );
        }

        // Step 6: decode class proto
        let class_result = Class::decode(&d1_bytes[stt_end..]);
        eprintln!(
            "Class decode from offset {stt_end} = {:?}",
            class_result.is_ok()
        );
        if let Err(ref e) = class_result {
            eprintln!("Class decode ERROR: {e}");
            // Try alternative offsets
            for offset in [0usize, 2, 3] {
                if offset < d1_bytes.len() {
                    let alt = Class::decode(&d1_bytes[offset..]);
                    eprintln!(
                        "  alt decode from offset {offset}: ok={}, err={:?}",
                        alt.is_ok(),
                        alt.as_ref().err()
                    );
                }
            }
        }
        let class_proto = class_result.unwrap_or_else(|_| {
            // Try from offset 0
            Class::decode(&d1_bytes[0..])
                .unwrap_or_else(|_| panic!("Cannot decode class proto from any offset"))
        });
        eprintln!(
            "fq_name={:?}, flags={:?}",
            class_proto.fq_name, class_proto.flags
        );
        eprintln!(
            "functions={}, properties={}, constructors={}",
            class_proto.function.len(),
            class_proto.property.len(),
            class_proto.constructor.len()
        );

        // Step 7: full extract
        let sigs = extract_kotlin_signatures(&raw);
        eprintln!("extract_kotlin_signatures = {:?}", sigs.is_some());
        if let Some(ref s) = sigs {
            eprintln!("class_display = {:?}", s.class_display);
            eprintln!("members count = {}", s.members.len());
            for m in &s.members {
                eprintln!(
                    "  {} {} -> {}",
                    m.jvm_name, m.jvm_descriptor, m.kotlin_display
                );
            }
        }
    }
}
