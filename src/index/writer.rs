//! Index creation, document insertion, and GAV-level deletion.
//!
//! # Indexing flow
//!
//! ```text
//! DependencyInfo
//!   ├─ source_jar_path ──▶ build_source_table() ──▶ SourceTable
//!   └─ jar_path ──▶ process_class_files()
//!                     └─ per .class:
//!                         extract_symbols() ──▶ Vec<SymbolDoc>
//!                           └─ lookup (package, source_file_name) in SourceTable
//!                               → set source (SourceOrigin::SourceJar)
//!                         add_symbol_doc() ──▶ Tantivy document
//! ```

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use rayon::prelude::*;
use tantivy::schema::*;
use tantivy::{Index, IndexWriter, doc};

use crate::manifest::DependencyInfo;
use crate::model::SymbolDoc;
use crate::parser::{classfile, jar};

use super::schema::build_schema;

/// Heap budget (in bytes) for the Tantivy `IndexWriter` (50 MB).
const HEAP_SIZE: usize = 50_000_000; // 50MB

/// Cached field handles for the Tantivy schema.
///
/// Resolves all 16 field names once at index-open time, avoiding repeated
/// `HashMap` lookups in `add_symbol_doc()` (which is called per symbol).
pub struct SchemaFields {
    /// GAV coordinates field.
    pub gav: Field,
    /// Symbol kind field (`class`, `method`, `field`).
    pub symbol_kind: Field,
    /// Fully qualified name field.
    pub fqn: Field,
    /// Java package name field.
    pub package: Field,
    /// Simple class name field.
    pub class_name: Field,
    /// Simple (unqualified) symbol name field.
    pub simple_name: Field,
    /// CamelCase-split tokens for search.
    pub name_parts: Field,
    /// Reversed simple name for suffix glob acceleration.
    pub simple_name_rev: Field,
    /// Reversed package name for suffix glob acceleration.
    pub package_rev: Field,
    /// Raw JVM descriptor field.
    pub descriptor: Field,
    /// Java-style signature field.
    pub signature_java: Field,
    /// Kotlin-style signature field.
    pub signature_kotlin: Field,
    /// Access modifier flags field.
    pub access_flags: Field,
    /// Visibility level field.
    pub access_level: Field,
    /// Source origin tag field.
    pub source: Field,
    /// Source JAR path field.
    pub source_path: Field,
    /// Source language field.
    pub source_language: Field,
    /// Source file name field.
    pub source_file_name: Field,
    /// Comma-separated classpaths (e.g. `"compile,runtime"`).
    pub classpaths: Field,
}

impl SchemaFields {
    /// Resolve all field handles from the given schema.
    ///
    /// # Panics
    ///
    /// Panics if any expected field is missing from the schema.
    pub fn new(schema: &Schema) -> Self {
        Self {
            gav: schema.get_field("gav").unwrap(),
            symbol_kind: schema.get_field("symbol_kind").unwrap(),
            fqn: schema.get_field("fqn").unwrap(),
            package: schema.get_field("package").unwrap(),
            class_name: schema.get_field("class_name").unwrap(),
            simple_name: schema.get_field("simple_name").unwrap(),
            name_parts: schema.get_field("name_parts").unwrap(),
            simple_name_rev: schema.get_field("simple_name_rev").unwrap(),
            package_rev: schema.get_field("package_rev").unwrap(),
            descriptor: schema.get_field("descriptor").unwrap(),
            signature_java: schema.get_field("signature_java").unwrap(),
            signature_kotlin: schema.get_field("signature_kotlin").unwrap(),
            access_flags: schema.get_field("access_flags").unwrap(),
            access_level: schema.get_field("access_level").unwrap(),
            source: schema.get_field("source").unwrap(),
            source_path: schema.get_field("source_path").unwrap(),
            source_language: schema.get_field("source_language").unwrap(),
            source_file_name: schema.get_field("source_file_name").unwrap(),
            classpaths: schema.get_field("classpaths").unwrap(),
        }
    }
}

/// Result of opening or creating a Tantivy index.
pub struct OpenIndexResult {
    /// The Tantivy index.
    pub index: Index,
    /// Whether the index was rebuilt due to an outdated schema.
    pub schema_rebuilt: bool,
}

/// Open or create a tantivy index at the given path.
///
/// If the existing index has an outdated schema (missing required fields),
/// the index directory is deleted and recreated with the current schema.
/// In that case, `schema_rebuilt` is set to `true` so the caller can
/// force a full re-index.
pub fn open_or_create_index(index_dir: &Path) -> Result<OpenIndexResult> {
    std::fs::create_dir_all(index_dir)?;
    let schema = build_schema();

    if index_dir.join("meta.json").exists() {
        let existing = Index::open_in_dir(index_dir).with_context(|| "opening existing index")?;
        if is_schema_compatible(&existing.schema()) {
            return Ok(OpenIndexResult {
                index: existing,
                schema_rebuilt: false,
            });
        }
        // Schema outdated — drop the old index and recreate
        eprintln!("Index schema outdated, rebuilding...");
        drop(existing);
        std::fs::remove_dir_all(index_dir)?;
        std::fs::create_dir_all(index_dir)?;
        let index =
            Index::create_in_dir(index_dir, schema).with_context(|| "creating new index")?;
        Ok(OpenIndexResult {
            index,
            schema_rebuilt: true,
        })
    } else {
        let index =
            Index::create_in_dir(index_dir, schema).with_context(|| "creating new index")?;
        Ok(OpenIndexResult {
            index,
            schema_rebuilt: false,
        })
    }
}

/// Check whether the index at the given path has a schema compatible with the
/// current version.  Returns `false` when the index does not exist or its
/// schema is missing required fields.
pub fn is_index_schema_current(index_dir: &Path) -> bool {
    if !index_dir.join("meta.json").exists() {
        return false;
    }
    match Index::open_in_dir(index_dir) {
        Ok(idx) => is_schema_compatible(&idx.schema()),
        Err(_) => false,
    }
}

/// Check whether the existing index schema contains all required fields.
fn is_schema_compatible(schema: &Schema) -> bool {
    super::compat::REQUIRED_FIELDS
        .iter()
        .all(|&name| schema.get_field(name).is_ok())
}

/// Delete all documents for a given GAV from the index.
pub fn delete_gav(writer: &IndexWriter, fields: &SchemaFields, gav: &str) -> Result<()> {
    let term = tantivy::Term::from_field_text(fields.gav, gav);
    writer.delete_term(term);
    Ok(())
}

/// Index all symbols from a single dependency JAR.
///
/// Collects all `.class` entries into memory, then parses and indexes them
/// in parallel using rayon.
pub fn index_dependency(
    writer: &IndexWriter,
    fields: &SchemaFields,
    dep: &DependencyInfo,
    classpaths: &str,
) -> Result<usize> {
    let gav = dep.gav();
    let dep_has_source = dep.source_jar_path.as_ref().is_some_and(|p| p.exists());

    // Build (package, filename) → path source table
    let source_table = if dep_has_source {
        jar::build_source_table(dep.source_jar_path.as_ref().unwrap()).unwrap_or_default()
    } else {
        jar::SourceTable::new()
    };

    let class_files = jar::collect_class_files(&dep.jar_path)?;
    let count = AtomicUsize::new(0);

    class_files
        .par_iter()
        .for_each(|(class_path, class_bytes)| {
            let symbols = match classfile::extract_symbols(class_bytes, &gav) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("  warning: failed to parse {class_path}: {e}");
                    return;
                }
            };

            for mut symbol in symbols {
                if dep_has_source && let Some(sfn) = symbol.source.source_file_name() {
                    let key = (symbol.package.clone(), sfn.to_string());
                    if let Some(entry) = source_table.get(&key) {
                        symbol.source = symbol.source.with_source_jar(entry.path.clone());
                    }
                }

                if let Err(e) = add_symbol_doc(writer, fields, &symbol, classpaths) {
                    eprintln!("  warning: failed to add symbol {}: {e}", symbol.fqn);
                }
                count.fetch_add(1, Ordering::Relaxed);
            }
        });

    Ok(count.load(Ordering::Relaxed))
}

fn add_symbol_doc(
    writer: &IndexWriter,
    f: &SchemaFields,
    doc_data: &SymbolDoc,
    classpaths: &str,
) -> Result<()> {
    let lang_str = doc_data
        .source
        .source_language()
        .map(|l| l.to_string())
        .unwrap_or_default();

    writer.add_document(doc!(
        f.gav => doc_data.gav.as_str(),
        f.symbol_kind => doc_data.symbol_kind.as_str(),
        f.fqn => doc_data.fqn.as_str(),
        f.package => doc_data.package.as_str(),
        f.class_name => doc_data.class_name.as_str(),
        f.simple_name => doc_data.simple_name.as_str(),
        f.name_parts => doc_data.name_parts.as_str(),
        f.simple_name_rev => crate::model::reverse_str(&doc_data.simple_name),
        f.package_rev => crate::model::reverse_str(&doc_data.package),
        f.descriptor => doc_data.descriptor.as_str(),
        f.signature_java => doc_data.signature.java.as_str(),
        f.signature_kotlin => doc_data.signature.kotlin.as_deref().unwrap_or(""),
        f.access_flags => doc_data.access_flags.as_str(),
        f.access_level => doc_data.access_level.as_str(),
        f.source => doc_data.source.as_str(),
        f.source_path => doc_data.source.source_path().unwrap_or(""),
        f.source_language => lang_str.as_str(),
        f.source_file_name => doc_data.source.source_file_name().unwrap_or(""),
        f.classpaths => classpaths,
    ))?;

    Ok(())
}

/// Create a new IndexWriter with the configured heap size.
pub fn create_writer(index: &Index) -> Result<IndexWriter> {
    index
        .writer(HEAP_SIZE)
        .with_context(|| "creating index writer")
}
