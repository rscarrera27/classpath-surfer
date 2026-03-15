//! Shared domain types and command output DTOs.
//!
//! Domain types (`SymbolDoc`, `SearchResult`, `SourceOrigin`, …) model the
//! indexing pipeline's data.  Command output types (`SearchOutput`,
//! `ShowOutput`, `StatusOutput`, …) are serializable DTOs returned by CLI
//! handlers.

/// Command output DTOs (`SearchOutput`, `ShowOutput`, `StatusOutput`, …).
pub mod output;

// Re-export everything so existing `use crate::model::*` paths keep working.
pub use output::*;

use serde::{Deserialize, Serialize};

/// Grouped signature representations for a symbol.
///
/// Every symbol has a Java-style signature derived from the classfile.  Kotlin
/// classes additionally carry a Kotlin-style signature decoded from
/// `@kotlin.Metadata`.  The two are bundled here so call-sites can pick the
/// right display form for the language the coding agent is currently using.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureDisplay {
    /// Human-readable Java-style signature (e.g. `public static ImmutableList of(Object arg0)`).
    #[serde(rename = "signature_java")]
    pub java: String,
    /// Kotlin-style signature (e.g. `suspend fun fetch(): Data`), present only for Kotlin classes.
    #[serde(rename = "signature_kotlin")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kotlin: Option<String>,
}

impl SignatureDisplay {
    /// Return the signature most appropriate for `lang`.
    ///
    /// When `lang` is `"kotlin"` and a Kotlin signature exists it is returned;
    /// otherwise the Java signature is used as a universal fallback.
    pub fn for_language(&self, lang: &str) -> &str {
        if lang == "kotlin" {
            self.kotlin.as_deref().unwrap_or(&self.java)
        } else {
            &self.java
        }
    }
}

/// The source language of a symbol, detected from the classfile's SourceFile attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceLanguage {
    /// Java source (`.java`).
    Java,
    /// Kotlin source (`.kt`).
    Kotlin,
    /// Scala source (`.scala`).
    Scala,
    /// Groovy source (`.groovy`).
    Groovy,
    /// Clojure source (`.clj`).
    Clojure,
    /// Unknown source language (SourceFile attribute present but unrecognized extension).
    Unknown,
}

impl std::fmt::Display for SourceLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceLanguage::Java => f.write_str("java"),
            SourceLanguage::Kotlin => f.write_str("kotlin"),
            SourceLanguage::Scala => f.write_str("scala"),
            SourceLanguage::Groovy => f.write_str("groovy"),
            SourceLanguage::Clojure => f.write_str("clojure"),
            SourceLanguage::Unknown => f.write_str("unknown"),
        }
    }
}

/// Source origin for a symbol — source JAR (with optional path) or decompiled.
///
/// Bundles all source-related metadata: origin tag, source JAR path, source
/// language (from the classfile SourceFile attribute), and source file name.
///
/// Serialized via `#[serde(flatten)]` into parent structs, producing a flat
/// `"source": "source_jar"` / `"source": "decompiled"` tag in JSON output.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum SourceOrigin {
    /// Source extracted from a source JAR.
    SourceJar {
        /// Path inside the source JAR (e.g. `com/google/common/collect/ImmutableList.java`).
        #[serde(skip_serializing_if = "Option::is_none")]
        source_path: Option<String>,
        /// Source language detected from the SourceFile attribute.
        #[serde(skip_serializing_if = "Option::is_none")]
        source_language: Option<SourceLanguage>,
        /// Original source file name (e.g. `"Foo.kt"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        source_file_name: Option<String>,
    },
    /// No source JAR available; source must be decompiled.
    Decompiled {
        /// Source language detected from the SourceFile attribute.
        #[serde(skip_serializing_if = "Option::is_none")]
        source_language: Option<SourceLanguage>,
        /// Original source file name (e.g. `"Foo.kt"`).
        #[serde(skip_serializing_if = "Option::is_none")]
        source_file_name: Option<String>,
    },
}

impl SourceOrigin {
    /// Whether source is from a source JAR.
    pub fn has_source(&self) -> bool {
        matches!(self, SourceOrigin::SourceJar { .. })
    }

    /// Path inside the source JAR, if available.
    pub fn source_path(&self) -> Option<&str> {
        match self {
            SourceOrigin::SourceJar { source_path, .. } => source_path.as_deref(),
            SourceOrigin::Decompiled { .. } => None,
        }
    }

    /// Source language detected from the classfile SourceFile attribute.
    pub fn source_language(&self) -> Option<SourceLanguage> {
        match self {
            SourceOrigin::SourceJar {
                source_language, ..
            }
            | SourceOrigin::Decompiled {
                source_language, ..
            } => *source_language,
        }
    }

    /// Original source file name from the classfile SourceFile attribute.
    pub fn source_file_name(&self) -> Option<&str> {
        match self {
            SourceOrigin::SourceJar {
                source_file_name, ..
            }
            | SourceOrigin::Decompiled {
                source_file_name, ..
            } => source_file_name.as_deref(),
        }
    }

    /// Tag string for index serialization (`"source_jar"` or `"decompiled"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceOrigin::SourceJar { .. } => "source_jar",
            SourceOrigin::Decompiled { .. } => "decompiled",
        }
    }

    /// Promote to `SourceJar` with the given path, preserving language and file name.
    pub fn with_source_jar(self, source_path: String) -> Self {
        let (sl, sfn) = match self {
            SourceOrigin::SourceJar {
                source_language,
                source_file_name,
                ..
            }
            | SourceOrigin::Decompiled {
                source_language,
                source_file_name,
            } => (source_language, source_file_name),
        };
        SourceOrigin::SourceJar {
            source_path: Some(source_path),
            source_language: sl,
            source_file_name: sfn,
        }
    }
}

/// Format a source language string (lowercase) into a capitalized display name.
///
/// Returns `"Java"` for unrecognized or missing language identifiers.
pub fn format_lang_display(lang: &str) -> &'static str {
    match lang {
        "kotlin" => "Kotlin",
        "scala" => "Scala",
        "groovy" => "Groovy",
        "clojure" => "Clojure",
        "unknown" => "Unknown",
        _ => "Java",
    }
}

/// The kind of symbol extracted from a classfile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    /// A class, interface, enum, or annotation type.
    Class,
    /// A method or constructor.
    Method,
    /// A field (instance or static).
    Field,
}

impl SymbolKind {
    /// Returns the lowercase string representation (`"class"`, `"method"`, or `"field"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Class => "class",
            SymbolKind::Method => "method",
            SymbolKind::Field => "field",
        }
    }
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A symbol extracted from a class file, ready to be indexed.
#[derive(Debug, Clone, Serialize)]
pub struct SymbolDoc {
    /// Maven GAV coordinates (`group:artifact:version`).
    pub gav: String,
    /// Whether this symbol is a class, method, or field.
    pub symbol_kind: SymbolKind,
    /// Fully qualified name (e.g. `com.google.common.collect.ImmutableList`).
    pub fqn: String,
    /// Java package name (e.g. `com.google.common.collect`).
    pub package: String,
    /// Simple class name, with inner-class `$` replaced by `.` (e.g. `Map.Entry`).
    pub class_name: String,
    /// Simple (unqualified) name of the symbol itself.
    pub simple_name: String,
    /// CamelCase-split tokens for search (e.g. `"Immutable List"` for `ImmutableList`).
    pub name_parts: String,
    /// Raw JVM type/method descriptor (e.g. `(ILjava/lang/String;)V`).
    pub descriptor: String,
    /// Human-readable signatures (Java and optional Kotlin).
    #[serde(flatten)]
    pub signature: SignatureDisplay,
    /// Java access modifiers as a space-separated string (e.g. `"public static final"`).
    pub access_flags: String,
    /// Visibility level: `"public"`, `"protected"`, `"private"`, or `"package_private"`.
    pub access_level: String,
    /// Source origin and metadata (source JAR path, language, file name).
    #[serde(flatten)]
    pub source: SourceOrigin,
}

/// A search result returned to the user.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    /// Maven GAV coordinates of the dependency containing this symbol.
    pub gav: String,
    /// Whether this symbol is a class, method, or field.
    pub symbol_kind: SymbolKind,
    /// Fully qualified name of the symbol.
    pub fqn: String,
    /// Simple (unqualified) name of the symbol.
    pub simple_name: String,
    /// Human-readable signatures (Java and optional Kotlin).
    #[serde(flatten)]
    pub signature: SignatureDisplay,
    /// Java access modifiers as a space-separated string.
    pub access_flags: String,
    /// Source origin tag: `"source_jar"` or `"decompiled"`.
    pub source: String,
    /// Source language detected from the classfile SourceFile attribute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_language: Option<SourceLanguage>,
}

impl SearchResult {
    /// Whether a source JAR is available for this symbol.
    pub fn has_source(&self) -> bool {
        self.source == "source_jar"
    }
}

/// Source code provided by a source JAR or decompiler.
#[derive(Debug)]
pub enum SourceProvider {
    /// Source extracted from a source JAR (Java or Kotlin).
    SourceJar {
        /// The source file content.
        content: String,
        /// Path inside the source JAR (e.g. `com/google/common/collect/ImmutableList.java`).
        path: String,
        /// Source language.
        language: SourceLanguage,
    },
    /// Source produced by an external decompiler (CFR or Vineflower).
    Decompiler {
        /// The decompiled source content.
        content: String,
    },
}

/// Resolved source code with optional secondary view.
///
/// When the primary source is non-Java, a secondary decompiled Java view may be provided.
#[derive(Debug)]
pub struct ResolvedSource {
    /// Maven GAV coordinates of the dependency containing this class.
    pub gav: String,
    /// Primary source (source JAR preferred over decompiler).
    pub primary: SourceProvider,
    /// Secondary source view (e.g. decompiler Java for non-Java sources).
    pub secondary: Option<SourceProvider>,
}
