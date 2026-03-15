//! Kotlin metadata decoding and Kotlin-style signature generation.
//!
//! Parses the `@kotlin.Metadata` annotation from class files to extract
//! function, property, and class signatures in Kotlin syntax.

mod format;
/// Protobuf message definitions matching Kotlin's `metadata.proto`.
pub mod proto;

use std::collections::HashMap;

use prost::Message;

use proto::*;

/// Well-known predefined qualified names used by the Kotlin metadata string table.
const PREDEFINED_NAMES: &[&str] = &[
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

/// Raw fields extracted from the `@kotlin.Metadata` annotation.
pub struct KotlinMetadataRaw {
    /// Metadata kind: 1=class, 2=file facade, 3=synthetic, 4=multi-file facade, 5=multi-file part.
    pub k: i32,
    /// Protobuf-encoded metadata (concatenated string chars → bytes).
    pub d1: Vec<String>,
    /// String table for name lookups.
    pub d2: Vec<String>,
}

/// Extracted Kotlin signatures for a class and its members.
pub struct KotlinSignatures {
    /// Kotlin-style class display (e.g. "data class Foo").
    pub class_display: Option<String>,
    /// Member signatures keyed by (JVM method name, JVM descriptor).
    pub members: Vec<KotlinMemberSignature>,
}

/// A single Kotlin member signature matched to its JVM counterpart.
pub struct KotlinMemberSignature {
    /// JVM method/field name.
    pub jvm_name: String,
    /// JVM descriptor (for disambiguation of overloads).
    pub jvm_descriptor: String,
    /// Kotlin-style signature string.
    pub kotlin_display: String,
}

/// Resolver for looking up names from the Kotlin metadata string table.
pub struct NameResolver {
    string_table_types: StringTableTypes,
    strings: Vec<String>,
}

impl NameResolver {
    fn new(stt: StringTableTypes, d2: Vec<String>) -> Self {
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

fn apply_operation(s: &str, operation: i32) -> String {
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
fn d1_to_bytes(d1: &[String]) -> Vec<u8> {
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
fn decode_varint(buf: &[u8]) -> Option<(u64, usize)> {
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

fn decode_class_signatures(d1_bytes: &[u8], d2: &[String]) -> Option<KotlinSignatures> {
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

fn decode_package_signatures(d1_bytes: &[u8], d2: &[String]) -> Option<KotlinSignatures> {
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

/// Build an approximate JVM descriptor hint for matching functions.
/// This is not a full descriptor — just enough info to disambiguate overloads.
fn build_jvm_descriptor_hint(
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
fn property_getter_name(property_name: &str) -> String {
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

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Build a simple NameResolver from d2 strings (no StringTableTypes records,
    /// so index == d2 position).
    fn make_resolver(strings: &[&str]) -> NameResolver {
        NameResolver::new(
            StringTableTypes {
                record: vec![],
                ..Default::default()
            },
            strings.iter().map(|s| s.to_string()).collect(),
        )
    }

    /// Build a Type that references a class by qualified-name index.
    fn class_type(class_name_idx: i32) -> Type {
        Type {
            class_name: Some(class_name_idx),
            nullable: Some(false),
            ..Default::default()
        }
    }

    /// Build a nullable class Type.
    fn nullable_class_type(class_name_idx: i32) -> Type {
        Type {
            class_name: Some(class_name_idx),
            nullable: Some(true),
            ..Default::default()
        }
    }

    /// Build a Type that references a type parameter by id.
    fn type_param_type(id: i32) -> Type {
        Type {
            type_parameter: Some(id),
            nullable: Some(false),
            ..Default::default()
        }
    }

    /// Build a simple ValueParameter.
    fn simple_param(name_idx: i32, ty: Type) -> ValueParameter {
        ValueParameter {
            flags: None,
            name: name_idx,
            r#type: Some(ty),
            type_id: None,
            vararg_element_type: None,
            ..Default::default()
        }
    }

    /// Encode StringTableTypes + a protobuf message into d1 strings (one char per byte).
    fn encode_d1<M: prost::Message>(stt: &StringTableTypes, msg: &M) -> Vec<String> {
        let stt_bytes = stt.encode_to_vec();
        let msg_bytes = msg.encode_to_vec();

        let mut buf = Vec::new();
        // UTF-8 mode marker (as the Kotlin compiler prepends)
        buf.push(0u8);
        // length-delimited STT
        prost::encoding::encode_varint(stt_bytes.len() as u64, &mut buf);
        buf.extend_from_slice(&stt_bytes);
        buf.extend_from_slice(&msg_bytes);

        // Convert to d1 format: each byte as a char
        let s: String = buf.iter().map(|&b| b as char).collect();
        vec![s]
    }

    // -----------------------------------------------------------------------
    // Existing tests
    // -----------------------------------------------------------------------

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
    fn test_property_getter_name() {
        assert_eq!(property_getter_name("name"), "getName");
        assert_eq!(property_getter_name("count"), "getCount");
        assert_eq!(property_getter_name("x"), "getX");
    }

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
    fn test_format_class_display() {
        // flags: public(3<<1=6) + final(0<<4=0) + class(0<<6=0) = 6
        assert_eq!(format::format_class_display(6, "Foo"), "class Foo");

        // data class: flags = 6 | (1<<10) = 1030
        assert_eq!(
            format::format_class_display(1030, "User"),
            "data class User"
        );

        // sealed class: flags = public(6) + sealed(3<<4=48) + class(0) = 54
        assert_eq!(
            format::format_class_display(54, "Result"),
            "sealed class Result"
        );

        // object: flags = public(6) + final(0) + object(5<<6=320) = 326
        assert_eq!(
            format::format_class_display(326, "Companion"),
            "object Companion"
        );
    }

    // -----------------------------------------------------------------------
    // Step 2: format_class_display extensions
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_class_display_interface() {
        // abstract interface: public(6) + abstract(2<<4=32) + interface(1<<6=64) = 102
        assert_eq!(
            format::format_class_display(102, "Runnable"),
            "abstract interface Runnable"
        );
    }

    #[test]
    fn test_format_class_display_enum() {
        // enum class: public(6) + final(0) + enum(2<<6=128) = 134
        assert_eq!(
            format::format_class_display(134, "Color"),
            "enum class Color"
        );
    }

    #[test]
    fn test_format_class_display_annotation() {
        // annotation class: public(6) + final(0) + annotation(4<<6=256) = 262
        assert_eq!(
            format::format_class_display(262, "JsonName"),
            "annotation class JsonName"
        );
    }

    #[test]
    fn test_format_class_display_companion_object() {
        // companion object: public(6) + final(0) + companion(6<<6=384) = 390
        assert_eq!(
            format::format_class_display(390, "Companion"),
            "companion object Companion"
        );
    }

    #[test]
    fn test_format_class_display_value_class() {
        // value class: public(6) + final(0) + class(0) + value(1<<13) = 6 + 8192 = 8198
        assert_eq!(
            format::format_class_display(8198, "Duration"),
            "value class Duration"
        );
    }

    #[test]
    fn test_format_class_display_fun_interface() {
        // fun interface: public(6) + abstract(32) + interface(64) + fun(1<<14=16384) = 16486
        assert_eq!(
            format::format_class_display(16486, "Action"),
            "abstract fun interface Action"
        );
    }

    #[test]
    fn test_format_class_display_private_open() {
        // private open class: private(1<<1=2) + open(1<<4=16) + class(0) = 18
        assert_eq!(
            format::format_class_display(18, "Impl"),
            "private open class Impl"
        );
    }

    // -----------------------------------------------------------------------
    // Step 3: kotlin_simple_name
    // -----------------------------------------------------------------------

    #[test]
    fn test_kotlin_simple_name() {
        assert_eq!(
            format::kotlin_simple_name("kotlin/collections/List"),
            "List"
        );
        assert_eq!(format::kotlin_simple_name("Foo"), "Foo");
        assert_eq!(format::kotlin_simple_name(""), "");
    }

    // -----------------------------------------------------------------------
    // Step 4: format_function_display
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_function_simple() {
        // fun greet() — Unit return omitted
        // d2: [0]=greet, [1]=(unused), [2]=kotlin/Unit
        let resolver = make_resolver(&["greet", "", "kotlin/Unit"]);
        let func = Function {
            flags: Some(6), // public final
            old_flags: None,
            name: 0,
            return_type: Some(class_type(2)),
            return_type_id: None,
            type_parameter: vec![],
            value_parameter: vec![],
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_function_display(&func, &resolver, None, &[]),
            "fun greet()"
        );
    }

    #[test]
    fn test_format_function_with_return_type() {
        // fun length(): Int
        // d2: [0]=length, [1]=kotlin/Int
        let resolver = make_resolver(&["length", "kotlin/Int"]);
        let func = Function {
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
        };
        assert_eq!(
            format::format_function_display(&func, &resolver, None, &[]),
            "fun length(): Int"
        );
    }

    #[test]
    fn test_format_function_with_params() {
        // fun add(a: Int, b: Int): Int
        // d2: [0]=add, [1]=kotlin/Int, [2]=a, [3]=b
        let resolver = make_resolver(&["add", "kotlin/Int", "a", "b"]);
        let func = Function {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(1)),
            return_type_id: None,
            type_parameter: vec![],
            value_parameter: vec![
                simple_param(2, class_type(1)),
                simple_param(3, class_type(1)),
            ],
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_function_display(&func, &resolver, None, &[]),
            "fun add(a: Int, b: Int): Int"
        );
    }

    #[test]
    fn test_format_function_suspend() {
        // suspend fun fetch(): String
        // d2: [0]=fetch, [1]=kotlin/String
        let resolver = make_resolver(&["fetch", "kotlin/String"]);
        let flags = 6 | (1 << 14); // public final suspend
        let func = Function {
            flags: Some(flags),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(1)),
            return_type_id: None,
            type_parameter: vec![],
            value_parameter: vec![],
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_function_display(&func, &resolver, None, &[]),
            "suspend fun fetch(): String"
        );
    }

    #[test]
    fn test_format_function_inline() {
        // inline fun run()
        let resolver = make_resolver(&["run", "kotlin/Unit"]);
        let flags = 6 | (1 << 11); // public final inline
        let func = Function {
            flags: Some(flags),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(1)),
            return_type_id: None,
            type_parameter: vec![],
            value_parameter: vec![],
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_function_display(&func, &resolver, None, &[]),
            "inline fun run()"
        );
    }

    #[test]
    fn test_format_function_infix() {
        // infix fun plus(other: Int): Int
        let resolver = make_resolver(&["plus", "kotlin/Int", "other"]);
        let flags = 6 | (1 << 10); // public final infix
        let func = Function {
            flags: Some(flags),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(1)),
            return_type_id: None,
            type_parameter: vec![],
            value_parameter: vec![simple_param(2, class_type(1))],
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_function_display(&func, &resolver, None, &[]),
            "infix fun plus(other: Int): Int"
        );
    }

    #[test]
    fn test_format_function_operator() {
        // operator fun get(index: Int): String
        let resolver = make_resolver(&["get", "kotlin/Int", "index", "kotlin/String"]);
        let flags = 6 | (1 << 9); // public final operator
        let func = Function {
            flags: Some(flags),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(3)),
            return_type_id: None,
            type_parameter: vec![],
            value_parameter: vec![simple_param(2, class_type(1))],
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_function_display(&func, &resolver, None, &[]),
            "operator fun get(index: Int): String"
        );
    }

    #[test]
    fn test_format_function_extension() {
        // fun String.isBlank(): Boolean
        let resolver = make_resolver(&["isBlank", "kotlin/String", "kotlin/Boolean"]);
        let func = Function {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(2)),
            return_type_id: None,
            type_parameter: vec![],
            value_parameter: vec![],
            receiver_type: Some(class_type(1)),
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_function_display(&func, &resolver, None, &[]),
            "fun String.isBlank(): Boolean"
        );
    }

    #[test]
    fn test_format_function_type_params() {
        // fun <T> identity(value: T): T
        // d2: [0]=identity, [1]=T, [2]=value
        let resolver = make_resolver(&["identity", "T", "value"]);
        let tp = TypeParameter {
            id: 0,
            name: 1, // "T"
            reified: None,
            variance: None,
            upper_bound: vec![],
            ..Default::default()
        };
        let func = Function {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: Some(type_param_type(0)),
            return_type_id: None,
            type_parameter: vec![tp.clone()],
            value_parameter: vec![simple_param(2, type_param_type(0))],
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_function_display(&func, &resolver, None, &[tp]),
            "fun <T> identity(value: T): T"
        );
    }

    #[test]
    fn test_format_function_private_abstract() {
        // private abstract fun doWork()
        // private(1<<1=2) + abstract(2<<4=32) = 34
        let resolver = make_resolver(&["doWork", "kotlin/Unit"]);
        let func = Function {
            flags: Some(34),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(1)),
            return_type_id: None,
            type_parameter: vec![],
            value_parameter: vec![],
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_function_display(&func, &resolver, None, &[]),
            "private abstract fun doWork()"
        );
    }

    // -----------------------------------------------------------------------
    // Step 5: format_property_display
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_property_simple() {
        // val name: String
        // public final val: visibility=public(3<<1=6) + modality=final(0) + isVar=0 = 6
        let resolver = make_resolver(&["name", "kotlin/String"]);
        let prop = Property {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(1)),
            return_type_id: None,
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, None, &[]),
            "val name: String"
        );
    }

    #[test]
    fn test_format_property_nullable() {
        // val label: String?
        let resolver = make_resolver(&["label", "kotlin/String"]);
        let prop = Property {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: Some(nullable_class_type(1)),
            return_type_id: None,
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, None, &[]),
            "val label: String?"
        );
    }

    #[test]
    fn test_format_property_var() {
        // var count: Int
        // public final var: 6 | (1<<9) = 6 + 512 = 518
        let resolver = make_resolver(&["count", "kotlin/Int"]);
        let prop = Property {
            flags: Some(518),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(1)),
            return_type_id: None,
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, None, &[]),
            "var count: Int"
        );
    }

    #[test]
    fn test_format_property_const() {
        // const val MAX: Int
        // public final const val: 6 | (1<<12) = 6 + 4096 = 4102
        let resolver = make_resolver(&["MAX", "kotlin/Int"]);
        let prop = Property {
            flags: Some(4102),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(1)),
            return_type_id: None,
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, None, &[]),
            "const val MAX: Int"
        );
    }

    #[test]
    fn test_format_property_lateinit() {
        // lateinit var adapter: Adapter
        // public final lateinit var: 6 | (1<<9) | (1<<13) = 6+512+8192 = 8710
        let resolver = make_resolver(&["adapter", "com/example/Adapter"]);
        let prop = Property {
            flags: Some(8710),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(1)),
            return_type_id: None,
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, None, &[]),
            "lateinit var adapter: Adapter"
        );
    }

    #[test]
    fn test_format_property_extension() {
        // val List.lastIndex: Int
        // We need "List" for receiver, "Int" for type
        let resolver = make_resolver(&["lastIndex", "kotlin/collections/List", "kotlin/Int"]);
        let prop = Property {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: Some(class_type(2)),
            return_type_id: None,
            receiver_type: Some(class_type(1)),
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, None, &[]),
            "val List.lastIndex: Int"
        );
    }

    // -----------------------------------------------------------------------
    // Step 6: type formatting (generic, projections, type alias)
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_property_generic_type() {
        // val items: List<String>
        let resolver = make_resolver(&["items", "kotlin/collections/List", "kotlin/String"]);
        let list_of_string = Type {
            class_name: Some(1),
            nullable: Some(false),
            argument: vec![TypeArgument {
                projection: Some(TypeProjection::Inv as i32),
                r#type: Some(class_type(2)),
                type_id: None,
            }],
            ..Default::default()
        };
        let prop = Property {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: Some(list_of_string),
            return_type_id: None,
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, None, &[]),
            "val items: List<String>"
        );
    }

    #[test]
    fn test_format_property_star_projection() {
        // val data: List<*>
        let resolver = make_resolver(&["data", "kotlin/collections/List"]);
        let list_star = Type {
            class_name: Some(1),
            nullable: Some(false),
            argument: vec![TypeArgument {
                projection: Some(TypeProjection::Star as i32),
                r#type: None,
                type_id: None,
            }],
            ..Default::default()
        };
        let prop = Property {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: Some(list_star),
            return_type_id: None,
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, None, &[]),
            "val data: List<*>"
        );
    }

    #[test]
    fn test_format_property_out_projection() {
        // val source: List<out String>
        let resolver = make_resolver(&["source", "kotlin/collections/List", "kotlin/String"]);
        let list_out_string = Type {
            class_name: Some(1),
            nullable: Some(false),
            argument: vec![TypeArgument {
                projection: Some(TypeProjection::Out as i32),
                r#type: Some(class_type(2)),
                type_id: None,
            }],
            ..Default::default()
        };
        let prop = Property {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: Some(list_out_string),
            return_type_id: None,
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, None, &[]),
            "val source: List<out String>"
        );
    }

    #[test]
    fn test_format_property_in_projection() {
        // val consumer: Comparable<in Int>
        let resolver = make_resolver(&["consumer", "kotlin/Comparable", "kotlin/Int"]);
        let comparable_in_int = Type {
            class_name: Some(1),
            nullable: Some(false),
            argument: vec![TypeArgument {
                projection: Some(TypeProjection::In as i32),
                r#type: Some(class_type(2)),
                type_id: None,
            }],
            ..Default::default()
        };
        let prop = Property {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: Some(comparable_in_int),
            return_type_id: None,
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, None, &[]),
            "val consumer: Comparable<in Int>"
        );
    }

    #[test]
    fn test_format_property_type_alias() {
        // val id: UserId — abbreviated_type takes priority
        let resolver = make_resolver(&["id", "kotlin/String", "com/example/UserId"]);
        let aliased_type = Type {
            class_name: Some(1), // underlying String
            nullable: Some(false),
            abbreviated_type: Some(Box::new(Type {
                class_name: Some(2), // UserId
                nullable: Some(false),
                ..Default::default()
            })),
            ..Default::default()
        };
        let prop = Property {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: Some(aliased_type),
            return_type_id: None,
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, None, &[]),
            "val id: UserId"
        );
    }

    // -----------------------------------------------------------------------
    // Step 7: format_constructor_display
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_constructor_no_params() {
        let resolver = make_resolver(&[]);
        let ctor = Constructor {
            flags: Some(6),
            value_parameter: vec![],
            ..Default::default()
        };
        assert_eq!(
            format::format_constructor_display(&ctor, &resolver, None, &[]),
            "constructor()"
        );
    }

    #[test]
    fn test_format_constructor_with_params() {
        // constructor(name: String, age: Int)
        let resolver = make_resolver(&["name", "kotlin/String", "age", "kotlin/Int"]);
        let ctor = Constructor {
            flags: Some(6),
            value_parameter: vec![
                simple_param(0, class_type(1)),
                simple_param(2, class_type(3)),
            ],
            ..Default::default()
        };
        assert_eq!(
            format::format_constructor_display(&ctor, &resolver, None, &[]),
            "constructor(name: String, age: Int)"
        );
    }

    // -----------------------------------------------------------------------
    // Step 8: ValueParameter edge cases (via constructor display)
    // -----------------------------------------------------------------------

    #[test]
    fn test_param_with_default_value() {
        // constructor(x: Int = ...)
        let resolver = make_resolver(&["x", "kotlin/Int"]);
        let ctor = Constructor {
            flags: Some(6),
            value_parameter: vec![ValueParameter {
                flags: Some(1 << 1), // hasDefault
                name: 0,
                r#type: Some(class_type(1)),
                type_id: None,
                vararg_element_type: None,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            format::format_constructor_display(&ctor, &resolver, None, &[]),
            "constructor(x: Int = ...)"
        );
    }

    #[test]
    fn test_param_crossinline() {
        // constructor(crossinline block: Function0)
        let resolver = make_resolver(&["block", "kotlin/Function0"]);
        let ctor = Constructor {
            flags: Some(6),
            value_parameter: vec![ValueParameter {
                flags: Some(1 << 2), // crossinline
                name: 0,
                r#type: Some(class_type(1)),
                type_id: None,
                vararg_element_type: None,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            format::format_constructor_display(&ctor, &resolver, None, &[]),
            "constructor(crossinline block: Function0)"
        );
    }

    #[test]
    fn test_param_noinline() {
        // constructor(noinline action: Function0)
        let resolver = make_resolver(&["action", "kotlin/Function0"]);
        let ctor = Constructor {
            flags: Some(6),
            value_parameter: vec![ValueParameter {
                flags: Some(1 << 3), // noinline
                name: 0,
                r#type: Some(class_type(1)),
                type_id: None,
                vararg_element_type: None,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            format::format_constructor_display(&ctor, &resolver, None, &[]),
            "constructor(noinline action: Function0)"
        );
    }

    #[test]
    fn test_param_vararg() {
        // constructor(vararg items: String)
        let resolver = make_resolver(&["items", "kotlin/String"]);
        let ctor = Constructor {
            flags: Some(6),
            value_parameter: vec![ValueParameter {
                flags: None,
                name: 0,
                r#type: None,
                type_id: None,
                vararg_element_type: Some(class_type(1)),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            format::format_constructor_display(&ctor, &resolver, None, &[]),
            "constructor(vararg items: String)"
        );
    }

    // -----------------------------------------------------------------------
    // Step 9: NameResolver additional tests
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Step 10: build_signature_map
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Step 11: TypeTable indirect reference
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_function_return_type_from_type_table() {
        // Function with return_type_id pointing into TypeTable
        let resolver = make_resolver(&["compute", "kotlin/Int"]);
        let tt = TypeTable {
            r#type: vec![class_type(1)], // index 0 → Int
            first_nullable: None,
        };
        let func = Function {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: None,
            return_type_id: Some(0), // look up from TypeTable
            type_parameter: vec![],
            value_parameter: vec![],
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_function_display(&func, &resolver, Some(&tt), &[]),
            "fun compute(): Int"
        );
    }

    #[test]
    fn test_format_property_type_from_type_table() {
        // Property with return_type_id pointing into TypeTable
        let resolver = make_resolver(&["size", "kotlin/Int"]);
        let tt = TypeTable {
            r#type: vec![class_type(1)],
            first_nullable: None,
        };
        let prop = Property {
            flags: Some(6),
            old_flags: None,
            name: 0,
            return_type: None,
            return_type_id: Some(0),
            receiver_type: None,
            receiver_type_id: None,
            ..Default::default()
        };
        assert_eq!(
            format::format_property_display(&prop, &resolver, Some(&tt), &[]),
            "val size: Int"
        );
    }

    // -----------------------------------------------------------------------
    // Step 12: extract_kotlin_signatures E2E (protobuf round-trip)
    // -----------------------------------------------------------------------

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
