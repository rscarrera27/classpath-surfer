//! Kotlin metadata decoding and Kotlin-style signature generation.
//!
//! Parses the `@kotlin.Metadata` annotation from class files to extract
//! function, property, and class signatures in Kotlin syntax.

mod decoder;
mod format;
mod name_resolver;
/// Protobuf message definitions matching Kotlin's `metadata.proto`.
pub mod proto;
mod signatures;

pub use decoder::extract_kotlin_signatures;
pub use name_resolver::NameResolver;
pub use signatures::build_signature_map;

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

#[cfg(test)]
pub(super) mod tests {
    use prost::Message;

    use super::name_resolver::NameResolver;
    use super::proto::*;
    use super::*;

    // -------------------------------------------------------------------
    // Shared test helpers (used by submodule tests)
    // -------------------------------------------------------------------

    /// Build a simple NameResolver from d2 strings (no StringTableTypes records,
    /// so index == d2 position).
    pub(super) fn make_resolver(strings: &[&str]) -> NameResolver {
        NameResolver::new(
            StringTableTypes {
                record: vec![],
                ..Default::default()
            },
            strings.iter().map(|s| s.to_string()).collect(),
        )
    }

    /// Build a Type that references a class by qualified-name index.
    pub(super) fn class_type(class_name_idx: i32) -> Type {
        Type {
            class_name: Some(class_name_idx),
            nullable: Some(false),
            ..Default::default()
        }
    }

    /// Build a nullable class Type.
    pub(super) fn nullable_class_type(class_name_idx: i32) -> Type {
        Type {
            class_name: Some(class_name_idx),
            nullable: Some(true),
            ..Default::default()
        }
    }

    /// Build a Type that references a type parameter by id.
    pub(super) fn type_param_type(id: i32) -> Type {
        Type {
            type_parameter: Some(id),
            nullable: Some(false),
            ..Default::default()
        }
    }

    /// Build a simple ValueParameter.
    pub(super) fn simple_param(name_idx: i32, ty: Type) -> ValueParameter {
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
    pub(super) fn encode_d1<M: prost::Message>(stt: &StringTableTypes, msg: &M) -> Vec<String> {
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

    // -------------------------------------------------------------------
    // Tests for format:: functions (they live here since they test the
    // public interface of the format submodule)
    // -------------------------------------------------------------------

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

    #[test]
    fn test_kotlin_simple_name() {
        assert_eq!(
            format::kotlin_simple_name("kotlin/collections/List"),
            "List"
        );
        assert_eq!(format::kotlin_simple_name("Foo"), "Foo");
        assert_eq!(format::kotlin_simple_name(""), "");
    }

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
}
