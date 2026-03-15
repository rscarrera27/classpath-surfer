/// Parse a JVM type descriptor into a human-readable type name.
///
/// # Examples
///
/// ```
/// use classpath_surfer::parser::descriptor::type_to_string;
///
/// assert_eq!(type_to_string("I"), "int");
/// assert_eq!(type_to_string("Ljava/lang/String;"), "String");
/// assert_eq!(type_to_string("[B"), "byte[]");
/// ```
pub fn type_to_string(desc: &str) -> String {
    let (result, _) = parse_type(desc);
    result
}

/// Parse a method descriptor into a human-readable signature.
///
/// Currently unused — reserved for future descriptor display features.
///
/// # Examples
///
/// ```
/// use classpath_surfer::parser::descriptor::method_to_string;
///
/// assert_eq!(method_to_string("()V"), "() → void");
/// assert_eq!(method_to_string("(ILjava/lang/String;)V"), "(int, String) → void");
/// ```
#[allow(dead_code)]
pub fn method_to_string(desc: &str) -> String {
    if !desc.starts_with('(') {
        return desc.to_string();
    }

    let close = match desc.find(')') {
        Some(i) => i,
        None => return desc.to_string(),
    };

    let params_str = &desc[1..close];
    let return_str = &desc[close + 1..];

    let params = parse_param_list(params_str);
    let ret = type_to_string(return_str);

    let params_display = params.join(", ");
    format!("({params_display}) → {ret}")
}

/// Build a display signature for a method.
/// E.g. "public static ImmutableList of(Object element)"
pub fn format_method_display(
    access: &str,
    class_simple_name: &str,
    method_name: &str,
    descriptor: &str,
) -> String {
    let is_constructor = method_name == "<init>";
    let display_name = if is_constructor {
        class_simple_name
    } else {
        method_name
    };

    let (params, ret) = if descriptor.starts_with('(') {
        let close = descriptor.find(')').unwrap_or(descriptor.len());
        let params = parse_param_list(&descriptor[1..close]);
        let ret = if close + 1 < descriptor.len() {
            type_to_string(&descriptor[close + 1..])
        } else {
            "void".to_string()
        };
        (params, ret)
    } else {
        (vec![], descriptor.to_string())
    };

    let params_display: Vec<String> = params
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{t} arg{i}"))
        .collect();
    let params_str = params_display.join(", ");

    if is_constructor {
        if access.is_empty() {
            format!("{display_name}({params_str})")
        } else {
            format!("{access} {display_name}({params_str})")
        }
    } else if access.is_empty() {
        format!("{ret} {display_name}({params_str})")
    } else {
        format!("{access} {ret} {display_name}({params_str})")
    }
}

/// Build a display signature for a field.
/// E.g. "public static final int MAX_VALUE"
pub fn format_field_display(access: &str, field_name: &str, descriptor: &str) -> String {
    let type_name = type_to_string(descriptor);
    if access.is_empty() {
        format!("{type_name} {field_name}")
    } else {
        format!("{access} {type_name} {field_name}")
    }
}

fn parse_type(desc: &str) -> (String, usize) {
    if desc.is_empty() {
        return ("void".to_string(), 0);
    }

    match desc.as_bytes()[0] {
        b'V' => ("void".to_string(), 1),
        b'Z' => ("boolean".to_string(), 1),
        b'B' => ("byte".to_string(), 1),
        b'C' => ("char".to_string(), 1),
        b'S' => ("short".to_string(), 1),
        b'I' => ("int".to_string(), 1),
        b'J' => ("long".to_string(), 1),
        b'F' => ("float".to_string(), 1),
        b'D' => ("double".to_string(), 1),
        b'[' => {
            let (inner, consumed) = parse_type(&desc[1..]);
            (format!("{inner}[]"), 1 + consumed)
        }
        b'L' => {
            let semi = desc.find(';').unwrap_or(desc.len());
            let class_name = &desc[1..semi];
            let simple = class_name
                .rsplit('/')
                .next()
                .unwrap_or(class_name)
                .replace('$', ".");
            (simple, semi + 1)
        }
        _ => (desc.to_string(), desc.len()),
    }
}

fn parse_param_list(params_str: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut pos = 0;
    while pos < params_str.len() {
        let (type_name, consumed) = parse_type(&params_str[pos..]);
        if consumed == 0 {
            break;
        }
        params.push(type_name);
        pos += consumed;
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitive_types() {
        assert_eq!(type_to_string("I"), "int");
        assert_eq!(type_to_string("Z"), "boolean");
        assert_eq!(type_to_string("V"), "void");
        assert_eq!(type_to_string("D"), "double");
    }

    #[test]
    fn test_object_type() {
        assert_eq!(type_to_string("Ljava/lang/String;"), "String");
        assert_eq!(
            type_to_string("Lcom/google/common/collect/ImmutableList;"),
            "ImmutableList"
        );
    }

    #[test]
    fn test_array_types() {
        assert_eq!(type_to_string("[B"), "byte[]");
        assert_eq!(type_to_string("[Ljava/lang/String;"), "String[]");
        assert_eq!(type_to_string("[[I"), "int[][]");
    }

    #[test]
    fn test_method_descriptor() {
        assert_eq!(method_to_string("()V"), "() → void");
        assert_eq!(
            method_to_string("(ILjava/lang/String;)V"),
            "(int, String) → void"
        );
        assert_eq!(
            method_to_string("(Ljava/lang/Object;)Ljava/util/List;"),
            "(Object) → List"
        );
    }

    #[test]
    fn test_format_method_display() {
        assert_eq!(
            format_method_display(
                "public static",
                "ImmutableList",
                "of",
                "(Ljava/lang/Object;)Lcom/google/common/collect/ImmutableList;"
            ),
            "public static ImmutableList of(Object arg0)"
        );
        assert_eq!(
            format_method_display("public", "ArrayList", "<init>", "()V"),
            "public ArrayList()"
        );
    }

    #[test]
    fn test_format_field_display() {
        assert_eq!(
            format_field_display("public static final", "MAX_VALUE", "I"),
            "public static final int MAX_VALUE"
        );
    }

    #[test]
    fn test_inner_class() {
        assert_eq!(type_to_string("Ljava/util/Map$Entry;"), "Map.Entry");
    }
}
