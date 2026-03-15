//! Locate method definitions via classfile `LineNumberTable`.
//!
//! The JVM classfile format stores a `LineNumberTable` attribute in each
//! method's `Code` attribute, mapping bytecode offsets to source line numbers.
//! The minimum line number across all entries gives the method's starting line
//! in the original source file.
//!
//! This approach is precise and language-agnostic — it works identically for
//! Java, Kotlin, Scala, and Groovy.  It does NOT work for abstract/native
//! methods (no `Code` attribute) or classes compiled without debug info.

use cafebabe::attributes::AttributeData;
use cafebabe::parse_class;

/// Extract the 1-based source line of a method from classfile bytes.
///
/// Parses the classfile, finds all methods matching `method_name`, and returns
/// the minimum line number from their `LineNumberTable` attributes.
///
/// Returns `None` if the method is not found, has no `Code` attribute
/// (abstract/native), or was compiled without debug line info (`-g:none`).
pub fn find_method_line_from_classfile(class_bytes: &[u8], method_name: &str) -> Option<usize> {
    let class = parse_class(class_bytes).ok()?;
    let mut min_line: Option<u16> = None;

    for method in &class.methods {
        if method.name.as_ref() != method_name {
            continue;
        }
        for attr in &method.attributes {
            if let AttributeData::Code(code) = &attr.data {
                for code_attr in &code.attributes {
                    if let AttributeData::LineNumberTable(entries) = &code_attr.data {
                        for entry in entries {
                            min_line = Some(match min_line {
                                Some(current) => current.min(entry.line_number),
                                None => entry.line_number,
                            });
                        }
                    }
                }
            }
        }
    }

    min_line.map(|n| n as usize)
}
