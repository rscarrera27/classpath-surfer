use std::path::Path;

use anyhow::{Context, Result};
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, Occur, QueryParser, RegexQuery, TermQuery};
use tantivy::schema::*;
use tantivy::{Index, ReloadPolicy};

use crate::error::CliError;
use crate::model::{SearchQuery, SearchResult, SignatureDisplay, SourceLanguage, SymbolKind};

/// Read-only handle to the Tantivy symbol index.
pub struct IndexReader {
    index: Index,
    reader: tantivy::IndexReader,
}

impl IndexReader {
    /// Open an existing Tantivy index from `index_dir`.
    ///
    /// Returns an error if the index schema is missing required fields,
    /// indicating that the index was built with an older version and needs
    /// to be rebuilt via `classpath-surfer refresh --full`.
    pub fn open(index_dir: &Path) -> Result<Self> {
        let index = Index::open_in_dir(index_dir)
            .with_context(|| format!("opening index at {}", index_dir.display()))?;

        // Validate schema compatibility
        let schema = index.schema();
        let required_fields = [
            "gav",
            "symbol_kind",
            "fqn",
            "simple_name",
            "name_parts",
            "signature_java",
            "signature_kotlin",
            "access_flags",
            "access_level",
            "source",
            "source_language",
        ];
        let missing: Vec<&str> = required_fields
            .iter()
            .filter(|&&name| schema.get_field(name).is_err())
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(CliError::resource_not_found(
                "INDEX_OUTDATED",
                format!(
                    "Index schema is outdated (missing fields: {}). \
                     Run `classpath-surfer refresh` to rebuild.",
                    missing.join(", ")
                ),
            )
            .with_suggested_command("classpath-surfer refresh")
            .into());
        }

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(Self { index, reader })
    }

    /// Search the symbol index and return ranked results.
    ///
    /// Supports three search modes (selected by the boolean flags in [`SearchQuery`]):
    ///
    /// - **Text search** (default) -- tokenized query against `simple_name` and
    ///   `name_parts` fields.
    /// - **FQN mode** (`fqn_mode = true`) -- exact match on the `fqn` field.
    /// - **Regex mode** (`regex_mode = true`) -- regex pattern matched against
    ///   `simple_name`.
    ///
    /// Results can be narrowed by `symbol_type` (`"class"`, `"method"`, `"field"`,
    /// or `"any"`), by an optional `dependency` GAV filter, and by access level
    /// (e.g. `&["public", "protected"]`).  Pass `None` for `access_levels` to
    /// include all visibility levels.
    pub fn search(&self, sq: &SearchQuery) -> Result<(Vec<SearchResult>, usize)> {
        let schema = self.index.schema();
        let searcher = self.reader.searcher();

        let query: Box<dyn tantivy::query::Query> = if sq.fqn_mode {
            // Exact FQN match
            let fqn_field = schema.get_field("fqn").unwrap();
            Box::new(TermQuery::new(
                tantivy::Term::from_field_text(fqn_field, sq.query),
                IndexRecordOption::Basic,
            ))
        } else if sq.regex_mode {
            // Regex search on simple_name
            let simple_name_field = schema.get_field("simple_name").unwrap();
            Box::new(RegexQuery::from_pattern(sq.query, simple_name_field)?)
        } else {
            // Text search on simple_name and name_parts
            let simple_name = schema.get_field("simple_name").unwrap();
            let name_parts = schema.get_field("name_parts").unwrap();
            let parser = QueryParser::for_index(&self.index, vec![simple_name, name_parts]);
            parser.parse_query(sq.query)?
        };

        // Apply filters
        let mut clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = vec![];
        clauses.push((Occur::Must, query));

        if sq.symbol_type != "any" {
            let kind_field = schema.get_field("symbol_kind").unwrap();
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    tantivy::Term::from_field_text(kind_field, sq.symbol_type),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        if let Some(dep) = sq.dependency {
            let gav_field = schema.get_field("gav").unwrap();
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    tantivy::Term::from_field_text(gav_field, dep),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        if let Some(levels) = sq.access_levels {
            let al_field = schema.get_field("access_level").unwrap();
            let level_clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = levels
                .iter()
                .map(|&level| {
                    (
                        Occur::Should,
                        Box::new(TermQuery::new(
                            tantivy::Term::from_field_text(al_field, level),
                            IndexRecordOption::Basic,
                        )) as Box<dyn tantivy::query::Query>,
                    )
                })
                .collect();
            clauses.push((Occur::Must, Box::new(BooleanQuery::new(level_clauses))));
        }

        let combined = BooleanQuery::new(clauses);
        let (top_docs, total_count) = searcher.search(
            &combined,
            &(TopDocs::with_limit(sq.limit).and_offset(sq.offset), Count),
        )?;

        let mut results = Vec::new();
        for (_score, doc_address) in top_docs {
            let doc: tantivy::TantivyDocument = searcher.doc(doc_address)?;
            results.push(doc_to_search_result(&schema, &doc));
        }

        Ok((results, total_count))
    }

    /// Return the total number of indexed symbol documents.
    pub fn count_symbols(&self) -> Result<usize> {
        let searcher = self.reader.searcher();
        let count = searcher.search(&AllQuery, &tantivy::collector::Count)?;
        Ok(count)
    }
}

fn doc_to_search_result(schema: &Schema, doc: &tantivy::TantivyDocument) -> SearchResult {
    let get_text = |field_name: &str| -> String {
        schema
            .get_field(field_name)
            .ok()
            .and_then(|field| doc.get_first(field))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    let kind_str = get_text("symbol_kind");
    let symbol_kind = match kind_str.as_str() {
        "class" => SymbolKind::Class,
        "method" => SymbolKind::Method,
        "field" => SymbolKind::Field,
        _ => SymbolKind::Class,
    };

    let source = get_text("source");

    let source_language = match get_text("source_language").as_str() {
        "java" => Some(SourceLanguage::Java),
        "kotlin" => Some(SourceLanguage::Kotlin),
        "scala" => Some(SourceLanguage::Scala),
        "groovy" => Some(SourceLanguage::Groovy),
        "clojure" => Some(SourceLanguage::Clojure),
        "unknown" => Some(SourceLanguage::Unknown),
        _ => None,
    };

    let kt_sig = get_text("signature_kotlin");
    let kotlin = if kt_sig.is_empty() {
        None
    } else {
        Some(kt_sig)
    };

    SearchResult {
        gav: get_text("gav"),
        symbol_kind,
        fqn: get_text("fqn"),
        simple_name: get_text("simple_name"),
        signature: SignatureDisplay {
            java: get_text("signature_java"),
            kotlin,
        },
        access_flags: get_text("access_flags"),
        source,
        source_language,
    }
}
