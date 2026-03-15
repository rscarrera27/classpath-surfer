//! JAR and classfile parsing for symbol extraction.
//!
//! Opens JAR archives, parses `.class` files via the `cafebabe` crate,
//! and decodes JVM type/method descriptors into human-readable signatures.

/// `.class` file parsing and symbol extraction via the `cafebabe` crate.
pub mod classfile;
/// JVM type and method descriptor decoding into human-readable signatures.
pub mod descriptor;
/// JAR archive traversal and entry extraction.
pub mod jar;
/// Kotlin metadata decoding and signature formatting.
pub mod kotlin_metadata;
