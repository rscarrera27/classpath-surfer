//! JAR archive traversal, entry extraction, and source table construction.
//!
//! # Key data structures
//!
//! - `SourceTable` = `HashMap<(package, filename), SourceEntry>` — maps the
//!   `(package, filename)` pair (derived from parsing `package` declarations
//!   inside a source JAR) to the JAR-internal entry path.  This is the
//!   deterministic join key that unifies all source-JAR layouts (standard Java
//!   package paths, KMP source-set prefixes, file facades, etc.).
//! - `SourceEntry { path, language }` — a single entry in the table.
//!
//! # Source table construction flow
//!
//! ```text
//! source JAR
//!   │  for each .kt / .java entry
//!   ▼
//! extract_package_declaration(content)   ──▶  package name
//! filename_from_path(entry_path)         ──▶  filename
//!   │
//!   ▼
//! SourceTable: (package, filename) → SourceEntry { path, language }
//! ```
//!
//! Collision policy: when the same `(package, filename)` appears under multiple
//! source-set directories, `jvmMain/` is preferred (JVM `actual` implementations
//! are more useful for this tool's context).

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use zip::ZipArchive;

use crate::model::SourceLanguage;

/// Process all .class files in a JAR, calling the handler for each.
pub fn process_class_files(
    jar_path: &Path,
    mut handler: impl FnMut(&str, &[u8]) -> Result<()>,
) -> Result<()> {
    let file = std::fs::File::open(jar_path)
        .with_context(|| format!("opening JAR {}", jar_path.display()))?;
    let mut archive =
        ZipArchive::new(file).with_context(|| format!("reading ZIP {}", jar_path.display()))?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();

        if name.ends_with(".class") && !name.starts_with("META-INF/") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            handler(&name, &buf)?;
        }
    }
    Ok(())
}

/// Extract a single file from a JAR by its path within the archive.
pub fn extract_entry(jar_path: &Path, entry_path: &str) -> Result<Vec<u8>> {
    let file = std::fs::File::open(jar_path)
        .with_context(|| format!("opening JAR {}", jar_path.display()))?;
    let mut archive = ZipArchive::new(file)?;
    let mut entry = archive
        .by_name(entry_path)
        .with_context(|| format!("entry '{}' not found in {}", entry_path, jar_path.display()))?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

/// List all source files in a source JAR (`.java`, `.kt`, `.scala`, `.groovy`, `.clj`).
pub fn list_source_files(jar_path: &Path) -> Result<Vec<String>> {
    let file = std::fs::File::open(jar_path)
        .with_context(|| format!("opening JAR {}", jar_path.display()))?;
    let mut archive = ZipArchive::new(file)?;
    let mut sources = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        let name = entry.name().to_string();
        if is_source_file(&name) {
            sources.push(name);
        }
    }
    Ok(sources)
}

/// A source file entry in a source JAR, keyed by `(package, filename)`.
#[derive(Debug, Clone)]
pub struct SourceEntry {
    /// Full path inside the JAR (e.g. `"commonMain/CoroutineScope.kt"`).
    pub path: String,
    /// Source language.
    pub language: SourceLanguage,
}

/// Source table mapping `(package, filename)` to JAR entry paths.
pub type SourceTable = HashMap<(String, String), SourceEntry>;

/// Build a `(package, filename) → entry_path` table from a source JAR.
///
/// Reads each `.kt`/`.java` file, parses the `package` declaration,
/// and maps `(package, filename)` to the JAR-internal path.
pub fn build_source_table(jar_path: &Path) -> Result<SourceTable> {
    let file = std::fs::File::open(jar_path)
        .with_context(|| format!("opening source JAR {}", jar_path.display()))?;
    let mut archive = ZipArchive::new(file)?;
    let mut table = SourceTable::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();

        let language = match language_from_extension(&name) {
            Some(lang) => lang,
            None => continue,
        };

        let filename = filename_from_path(&name).to_string();

        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        let package = extract_package_declaration(&buf);

        let key = (package, filename);

        // Prefer jvmMain/ paths for JVM projects (actual implementations)
        if let Some(existing) = table.get(&key) {
            if !existing.path.starts_with("jvmMain/") && name.starts_with("jvmMain/") {
                table.insert(
                    key,
                    SourceEntry {
                        path: name,
                        language,
                    },
                );
            }
            // Otherwise keep the first entry
        } else {
            table.insert(
                key,
                SourceEntry {
                    path: name,
                    language,
                },
            );
        }
    }

    Ok(table)
}

/// Extract a package declaration from source file content.
///
/// Handles block comments, line comments, `@file:` annotations, and BOM.
/// Returns an empty string if no package declaration is found.
///
/// # Examples
///
/// ```
/// use classpath_surfer::parser::jar::extract_package_declaration;
///
/// assert_eq!(extract_package_declaration(b"package com.example;"), "com.example");
/// assert_eq!(extract_package_declaration(b"package kotlinx.coroutines"), "kotlinx.coroutines");
/// assert_eq!(extract_package_declaration(b"import foo\nclass Bar"), "");
/// ```
pub fn extract_package_declaration(content: &[u8]) -> String {
    // Handle UTF-8 BOM
    let text = if content.starts_with(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(&content[3..])
    } else {
        String::from_utf8_lossy(content)
    };

    let mut in_block_comment = false;

    for line in text.lines() {
        let line = line.trim();

        if in_block_comment {
            if let Some(pos) = line.find("*/") {
                // Rest of line after closing comment
                in_block_comment = false;
                let rest = line[pos + 2..].trim();
                if rest.is_empty() {
                    continue;
                }
                if let Some(pkg) = try_parse_package(rest) {
                    return pkg;
                }
                if is_code_start(rest) {
                    return String::new();
                }
            }
            continue;
        }

        // Check for block comment opening
        if let Some(after_open) = line.strip_prefix("/*") {
            if let Some(pos) = after_open.find("*/") {
                // Single-line block comment: /* ... */
                let rest = after_open[pos + 2..].trim();
                if rest.is_empty() {
                    continue;
                }
                if let Some(pkg) = try_parse_package(rest) {
                    return pkg;
                }
                if is_code_start(rest) {
                    return String::new();
                }
            } else {
                in_block_comment = true;
            }
            continue;
        }

        // Skip blank lines and line comments
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        // Skip @file: annotations (Kotlin)
        if line.starts_with('@') {
            continue;
        }

        // Try to parse package declaration
        if let Some(pkg) = try_parse_package(line) {
            return pkg;
        }

        // Any other code line means no package declaration
        if is_code_start(line) {
            return String::new();
        }
    }

    String::new()
}

/// Try to parse a `package ...` declaration from a line.
fn try_parse_package(line: &str) -> Option<String> {
    let stripped = line.strip_prefix("package ")?;
    // Remove trailing semicolon (Java) and trim
    Some(stripped.trim_end_matches(';').trim().to_string())
}

/// Check if a line starts actual code (not a package declaration).
fn is_code_start(line: &str) -> bool {
    line.starts_with("import ")
        || line.starts_with("class ")
        || line.starts_with("interface ")
        || line.starts_with("fun ")
        || line.starts_with("object ")
        || line.starts_with("enum ")
        || line.starts_with("abstract ")
        || line.starts_with("public ")
        || line.starts_with("private ")
        || line.starts_with("protected ")
        || line.starts_with("internal ")
        || line.starts_with("open ")
        || line.starts_with("sealed ")
        || line.starts_with("data ")
        || line.starts_with("typealias ")
}

/// Check if a file name has a recognized source extension.
fn is_source_file(name: &str) -> bool {
    name.ends_with(".java")
        || name.ends_with(".kt")
        || name.ends_with(".scala")
        || name.ends_with(".groovy")
        || name.ends_with(".clj")
}

/// Determine the source language from a file extension.
fn language_from_extension(name: &str) -> Option<SourceLanguage> {
    if name.ends_with(".kt") {
        Some(SourceLanguage::Kotlin)
    } else if name.ends_with(".java") {
        Some(SourceLanguage::Java)
    } else if name.ends_with(".scala") {
        Some(SourceLanguage::Scala)
    } else if name.ends_with(".groovy") {
        Some(SourceLanguage::Groovy)
    } else if name.ends_with(".clj") {
        Some(SourceLanguage::Clojure)
    } else {
        None
    }
}

/// Extract the filename from a JAR entry path.
fn filename_from_path(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Convert a fully qualified name to a source path stem (without extension).
///
/// For inner classes, returns the outer class path.
///
/// # Examples
///
/// ```
/// use classpath_surfer::parser::jar::fqn_to_source_stem;
///
/// assert_eq!(fqn_to_source_stem("com.google.common.collect.ImmutableList"), "com/google/common/collect/ImmutableList");
/// assert_eq!(fqn_to_source_stem("com.google.common.collect.ImmutableList.Builder"), "com/google/common/collect/ImmutableList");
/// assert_eq!(fqn_to_source_stem("Foo"), "Foo");
/// ```
pub fn fqn_to_source_stem(fqn: &str) -> String {
    let parts: Vec<&str> = fqn.split('.').collect();
    let mut class_start = parts.len() - 1;
    for (i, part) in parts.iter().enumerate() {
        if part.chars().next().is_some_and(|c| c.is_uppercase()) {
            class_start = i;
            break;
        }
    }

    let package_path = parts[..class_start].join("/");
    let outer_class = parts[class_start];

    if package_path.is_empty() {
        outer_class.to_string()
    } else {
        format!("{package_path}/{outer_class}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_package_java() {
        assert_eq!(
            extract_package_declaration(b"package com.example;"),
            "com.example"
        );
    }

    #[test]
    fn test_extract_package_kotlin() {
        assert_eq!(
            extract_package_declaration(b"package kotlinx.coroutines"),
            "kotlinx.coroutines"
        );
    }

    #[test]
    fn test_extract_package_after_line_comment() {
        assert_eq!(
            extract_package_declaration(b"// license\npackage foo"),
            "foo"
        );
    }

    #[test]
    fn test_extract_package_after_block_comment() {
        assert_eq!(
            extract_package_declaration(b"/* long license block */\npackage foo.bar;"),
            "foo.bar"
        );
    }

    #[test]
    fn test_extract_package_after_multiline_block_comment() {
        assert_eq!(
            extract_package_declaration(b"/*\n * line1\n * line2\n */\npackage baz"),
            "baz"
        );
    }

    #[test]
    fn test_extract_package_after_annotation() {
        assert_eq!(
            extract_package_declaration(b"@file:JvmName(\"Utils\")\npackage foo"),
            "foo"
        );
    }

    #[test]
    fn test_extract_package_no_package_import() {
        assert_eq!(extract_package_declaration(b"import foo\nclass Bar"), "");
    }

    #[test]
    fn test_extract_package_empty() {
        assert_eq!(extract_package_declaration(b""), "");
    }

    #[test]
    fn test_extract_package_bom() {
        assert_eq!(
            extract_package_declaration(b"\xEF\xBB\xBFpackage foo"),
            "foo"
        );
    }

    #[test]
    fn test_extract_package_inline_block_comment() {
        // Single-line block comment followed by package on next line
        assert_eq!(
            extract_package_declaration(b"/* copyright */\npackage com.example;"),
            "com.example"
        );
    }

    #[test]
    fn test_filename_from_path() {
        assert_eq!(
            filename_from_path("commonMain/CoroutineScope.kt"),
            "CoroutineScope.kt"
        );
        assert_eq!(
            filename_from_path("com/google/common/ImmutableList.java"),
            "ImmutableList.java"
        );
        assert_eq!(filename_from_path("Foo.kt"), "Foo.kt");
    }

    #[test]
    fn test_build_source_table_from_zip() {
        use std::io::Write;
        use zip::ZipWriter;
        use zip::write::SimpleFileOptions;

        let dir = tempfile::tempdir().unwrap();
        let jar_path = dir.path().join("sources.jar");

        // Create a test source JAR
        let file = std::fs::File::create(&jar_path).unwrap();
        let mut zip = ZipWriter::new(file);

        // Standard Java source
        zip.start_file("com/example/Foo.java", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"package com.example;\n\npublic class Foo {}")
            .unwrap();

        // KMP flat source
        zip.start_file("commonMain/Bar.kt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"package kotlinx.coroutines\n\nclass Bar")
            .unwrap();

        // KMP subpackage source
        zip.start_file(
            "commonMain/channels/Channel.kt",
            SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"package kotlinx.coroutines.channels\n\ninterface Channel")
            .unwrap();

        zip.finish().unwrap();

        let table = build_source_table(&jar_path).unwrap();

        // Standard Java
        let entry = table
            .get(&("com.example".to_string(), "Foo.java".to_string()))
            .unwrap();
        assert_eq!(entry.path, "com/example/Foo.java");
        assert_eq!(entry.language, SourceLanguage::Java);

        // KMP flat
        let entry = table
            .get(&("kotlinx.coroutines".to_string(), "Bar.kt".to_string()))
            .unwrap();
        assert_eq!(entry.path, "commonMain/Bar.kt");
        assert_eq!(entry.language, SourceLanguage::Kotlin);

        // KMP subpackage
        let entry = table
            .get(&(
                "kotlinx.coroutines.channels".to_string(),
                "Channel.kt".to_string(),
            ))
            .unwrap();
        assert_eq!(entry.path, "commonMain/channels/Channel.kt");
        assert_eq!(entry.language, SourceLanguage::Kotlin);
    }

    #[test]
    fn test_build_source_table_jvm_priority() {
        use std::io::Write;
        use zip::ZipWriter;
        use zip::write::SimpleFileOptions;

        let dir = tempfile::tempdir().unwrap();
        let jar_path = dir.path().join("sources.jar");

        let file = std::fs::File::create(&jar_path).unwrap();
        let mut zip = ZipWriter::new(file);

        // commonMain version
        zip.start_file("commonMain/Scope.kt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"package kotlinx.coroutines\n\nexpect class Scope")
            .unwrap();

        // jvmMain version (should win)
        zip.start_file("jvmMain/Scope.kt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"package kotlinx.coroutines\n\nactual class Scope")
            .unwrap();

        zip.finish().unwrap();

        let table = build_source_table(&jar_path).unwrap();
        let entry = table
            .get(&("kotlinx.coroutines".to_string(), "Scope.kt".to_string()))
            .unwrap();
        assert_eq!(entry.path, "jvmMain/Scope.kt");
    }
}
