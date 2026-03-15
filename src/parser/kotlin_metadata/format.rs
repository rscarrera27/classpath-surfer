//! Kotlin signature formatting from decoded metadata.

use super::NameResolver;
use super::proto::*;

/// Format a Kotlin class signature from class flags.
pub fn format_class_display(flags: i32, simple_name: &str) -> String {
    let kind = class_kind(flags);
    let modality = modality_str(flags);
    let visibility = visibility_str(flags);
    let is_data = flags & (1 << 10) != 0;
    let is_inner = flags & (1 << 9) != 0;
    let is_value = flags & (1 << 13) != 0;
    let is_fun_interface = flags & (1 << 14) != 0;

    let mut parts = Vec::new();
    if !visibility.is_empty() && visibility != "public" {
        parts.push(visibility);
    }
    if !modality.is_empty() && modality != "final" && kind != "object" {
        parts.push(modality);
    }
    if is_inner {
        parts.push("inner");
    }
    if is_data {
        parts.push("data");
    }
    if is_value {
        parts.push("value");
    }
    if is_fun_interface {
        parts.push("fun");
    }
    parts.push(kind);
    parts.push(simple_name);

    parts.join(" ")
}

/// Format a Kotlin function signature.
pub fn format_function_display(
    func: &Function,
    resolver: &NameResolver,
    type_table: Option<&TypeTable>,
    type_params: &[TypeParameter],
) -> String {
    let flags = func.flags.or(func.old_flags).unwrap_or(6);
    let is_suspend = flags & (1 << 14) != 0;
    let is_operator = flags & (1 << 9) != 0;
    let is_infix = flags & (1 << 10) != 0;
    let is_inline = flags & (1 << 11) != 0;
    let visibility = visibility_str(flags);
    let modality = modality_str(flags);

    let name = resolver.get_string(func.name);

    let mut parts = Vec::new();
    if !visibility.is_empty() && visibility != "public" {
        parts.push(visibility.to_string());
    }
    if !modality.is_empty() && modality != "final" {
        parts.push(modality.to_string());
    }
    if is_inline {
        parts.push("inline".to_string());
    }
    if is_suspend {
        parts.push("suspend".to_string());
    }
    if is_infix {
        parts.push("infix".to_string());
    }
    if is_operator {
        parts.push("operator".to_string());
    }
    parts.push("fun".to_string());

    // Type parameters
    let tp_str = format_type_params(&func.type_parameter, resolver);
    if !tp_str.is_empty() {
        parts.push(tp_str);
    }

    // Receiver type (extension function)
    let receiver = resolve_type_ref(&func.receiver_type, func.receiver_type_id, type_table);
    let receiver_str = receiver.map(|t| format_type(t, resolver, type_params));

    // Function name with receiver
    let name_with_receiver = if let Some(ref recv) = receiver_str {
        format!("{recv}.{name}")
    } else {
        name.to_string()
    };

    // Parameters
    let params: Vec<String> = func
        .value_parameter
        .iter()
        .map(|p| format_value_param(p, resolver, type_table, type_params))
        .collect();

    let params_str = params.join(", ");

    // Return type
    let ret_type = resolve_type_ref(&func.return_type, func.return_type_id, type_table);
    let ret_str = ret_type
        .map(|t| format_type(t, resolver, type_params))
        .unwrap_or_else(|| "Unit".to_string());

    parts.push(format!("{name_with_receiver}({params_str})"));

    let prefix = parts.join(" ");
    if ret_str == "Unit" {
        prefix
    } else {
        format!("{prefix}: {ret_str}")
    }
}

/// Format a Kotlin property signature.
pub fn format_property_display(
    prop: &Property,
    resolver: &NameResolver,
    type_table: Option<&TypeTable>,
    type_params: &[TypeParameter],
) -> String {
    let flags = prop.flags.or(prop.old_flags).unwrap_or(518);
    let is_var = flags & (1 << 9) != 0;
    let is_const = flags & (1 << 12) != 0;
    let is_lateinit = flags & (1 << 13) != 0;
    let visibility = visibility_str(flags);
    let modality = modality_str(flags);

    let name = resolver.get_string(prop.name);

    let mut parts = Vec::new();
    if !visibility.is_empty() && visibility != "public" {
        parts.push(visibility.to_string());
    }
    if !modality.is_empty() && modality != "final" {
        parts.push(modality.to_string());
    }
    if is_const {
        parts.push("const".to_string());
    }
    if is_lateinit {
        parts.push("lateinit".to_string());
    }
    parts.push(if is_var { "var" } else { "val" }.to_string());

    // Receiver type (extension property)
    let receiver = resolve_type_ref(&prop.receiver_type, prop.receiver_type_id, type_table);
    let receiver_str = receiver.map(|t| format_type(t, resolver, type_params));

    let name_with_receiver = if let Some(ref recv) = receiver_str {
        format!("{recv}.{name}")
    } else {
        name.to_string()
    };

    let ret_type = resolve_type_ref(&prop.return_type, prop.return_type_id, type_table);
    let type_str = ret_type
        .map(|t| format_type(t, resolver, type_params))
        .unwrap_or_else(|| "Any".to_string());

    parts.push(format!("{name_with_receiver}: {type_str}"));
    parts.join(" ")
}

/// Format a Kotlin constructor signature.
pub fn format_constructor_display(
    ctor: &Constructor,
    resolver: &NameResolver,
    type_table: Option<&TypeTable>,
    type_params: &[TypeParameter],
) -> String {
    let params: Vec<String> = ctor
        .value_parameter
        .iter()
        .map(|p| format_value_param(p, resolver, type_table, type_params))
        .collect();

    format!("constructor({})", params.join(", "))
}

fn format_value_param(
    param: &ValueParameter,
    resolver: &NameResolver,
    type_table: Option<&TypeTable>,
    type_params: &[TypeParameter],
) -> String {
    let name = resolver.get_string(param.name);
    let flags = param.flags.unwrap_or(0);
    let has_default = flags & (1 << 1) != 0;
    let is_crossinline = flags & (1 << 2) != 0;
    let is_noinline = flags & (1 << 3) != 0;

    let type_ref = if param.vararg_element_type.is_some() {
        // vararg parameter — show the element type
        param.vararg_element_type.as_ref()
    } else {
        resolve_type_ref(&param.r#type, param.type_id, type_table)
    };

    let type_str = type_ref
        .map(|t| format_type(t, resolver, type_params))
        .unwrap_or_else(|| "Any".to_string());

    let mut result = String::new();
    if is_crossinline {
        result.push_str("crossinline ");
    }
    if is_noinline {
        result.push_str("noinline ");
    }
    if param.vararg_element_type.is_some() {
        result.push_str("vararg ");
    }
    result.push_str(&format!("{name}: {type_str}"));
    if has_default {
        result.push_str(" = ...");
    }
    result
}

fn format_type(ty: &Type, resolver: &NameResolver, type_params: &[TypeParameter]) -> String {
    // Check abbreviated type first (type aliases)
    if let Some(ref abbrev) = ty.abbreviated_type {
        return format_type(abbrev, resolver, type_params);
    }

    let base = if let Some(class_name_idx) = ty.class_name {
        let qname = resolver.get_qualified_name(class_name_idx);
        kotlin_simple_name(&qname)
    } else if let Some(tp_id) = ty.type_parameter {
        // Look up type parameter name by ID
        type_params
            .iter()
            .find(|p| p.id == tp_id)
            .map(|p| resolver.get_string(p.name).to_string())
            .unwrap_or_else(|| format!("T{tp_id}"))
    } else if let Some(tp_name_idx) = ty.type_parameter_name {
        resolver.get_string(tp_name_idx).to_string()
    } else {
        "Any".to_string()
    };

    // Type arguments
    let args_str = if ty.argument.is_empty() {
        String::new()
    } else {
        let args: Vec<String> = ty
            .argument
            .iter()
            .map(|arg| format_type_argument(arg, resolver, type_params))
            .collect();
        format!("<{}>", args.join(", "))
    };

    let nullable = if ty.nullable.unwrap_or(false) {
        "?"
    } else {
        ""
    };

    format!("{base}{args_str}{nullable}")
}

fn format_type_argument(
    arg: &TypeArgument,
    resolver: &NameResolver,
    type_params: &[TypeParameter],
) -> String {
    let projection = arg.projection.unwrap_or(2); // default INV
    if projection == TypeProjection::Star as i32 {
        return "*".to_string();
    }

    let type_str = arg
        .r#type
        .as_ref()
        .map(|t| format_type(t, resolver, type_params))
        .unwrap_or_else(|| "Any".to_string());

    match projection {
        p if p == TypeProjection::In as i32 => format!("in {type_str}"),
        p if p == TypeProjection::Out as i32 => format!("out {type_str}"),
        _ => type_str, // INV — no prefix
    }
}

fn format_type_params(params: &[TypeParameter], resolver: &NameResolver) -> String {
    if params.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = params
        .iter()
        .map(|p| {
            let name = resolver.get_string(p.name);
            let variance = match p.variance.unwrap_or(2) {
                v if v == TypeVariance::In as i32 => "in ",
                v if v == TypeVariance::Out as i32 => "out ",
                _ => "",
            };
            format!("{variance}{name}")
        })
        .collect();
    format!("<{}>", parts.join(", "))
}

fn resolve_type_ref<'a>(
    direct: &'a Option<Type>,
    type_id: Option<i32>,
    type_table: Option<&'a TypeTable>,
) -> Option<&'a Type> {
    if let Some(t) = direct {
        return Some(t);
    }
    if let (Some(id), Some(table)) = (type_id, type_table) {
        return table.r#type.get(id as usize);
    }
    None
}

fn class_kind(flags: i32) -> &'static str {
    match (flags >> 6) & 0x7 {
        0 => "class",
        1 => "interface",
        2 => "enum class",
        3 => "enum entry",
        4 => "annotation class",
        5 => "object",
        6 => "companion object",
        _ => "class",
    }
}

fn visibility_str(flags: i32) -> &'static str {
    match (flags >> 1) & 0x7 {
        0 => "internal",
        1 => "private",
        2 => "protected",
        3 => "public",
        4 => "private",
        5 => "local",
        _ => "",
    }
}

fn modality_str(flags: i32) -> &'static str {
    match (flags >> 4) & 0x3 {
        0 => "final",
        1 => "open",
        2 => "abstract",
        3 => "sealed",
        _ => "",
    }
}

/// Convert a Kotlin qualified name to its simple name.
///
/// "kotlin/collections/List" → "List"
/// "kotlin/Int" → "Int"
pub(super) fn kotlin_simple_name(qname: &str) -> String {
    qname.rsplit('/').next().unwrap_or(qname).to_string()
}
