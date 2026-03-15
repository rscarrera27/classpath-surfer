use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, Occur, QueryParser, RegexQuery, TermQuery};
use tantivy::schema::*;
use tantivy::{Index, ReloadPolicy};

use crate::error::CliError;
use crate::model::{
    SearchQuery, SearchResult, SignatureDisplay, SourceLanguage, SymbolKind, matches_gav_pattern,
};

/// Result of GAV filter construction: the Tantivy query clause (if any) and matched GAVs.
type GavFilter = (Option<Box<dyn tantivy::query::Query>>, Vec<String>);

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
    /// to be rebuilt via `classpath-surfer refresh --force`.
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
            "scopes",
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
    /// When `query` is `Some`, supports three search modes:
    ///
    /// - **Smart search** (default) -- auto-detects FQN queries (2+ dots) for exact
    ///   match, otherwise token search on `simple_name` and `name_parts` with AND
    ///   semantics for multi-word queries.
    /// - **FQN mode** (`fqn_mode = true`) -- exact match on the `fqn` field.
    /// - **Regex mode** (`regex_mode = true`) -- regex pattern matched against
    ///   `simple_name`.
    ///
    /// When `query` is `None`, all symbols are returned (requires `dependency`).
    /// Results without a text query are sorted by kind then FQN.
    ///
    /// Results can be narrowed by `symbol_type` (comma-separated kinds like
    /// `"class,method"`, or `"any"` for all), by a `dependency` GAV pattern
    /// (glob with `*` wildcards), and by access level.
    ///
    /// Returns `(results, total_count, matched_gavs)` where `matched_gavs` is
    /// `Some` when a `dependency` pattern was provided.
    pub fn search(
        &self,
        sq: &SearchQuery,
    ) -> Result<(Vec<SearchResult>, usize, Option<Vec<String>>)> {
        let schema = self.index.schema();
        let searcher = self.reader.searcher();
        let is_listing = sq.query.is_none();

        let mut clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = vec![];

        // Base query (search mode classification)
        clauses.push((Occur::Must, build_base_query(&self.index, &schema, sq)?));

        // Symbol type filter
        if let Some(filter) = build_symbol_type_filter(&schema, sq.symbol_type) {
            clauses.push((Occur::Must, filter));
        }

        // Dependency GAV filter
        let matched_gavs = if let Some(dep) = sq.dependency {
            let (filter, gavs) = self.build_gav_filter(&schema, dep)?;
            if let Some(filter) = filter {
                clauses.push((Occur::Must, filter));
            } else {
                // Empty glob match — no results possible
                return Ok((vec![], 0, Some(vec![])));
            }
            Some(gavs)
        } else {
            None
        };

        // Access level filter
        if let Some(filter) = build_access_level_filter(&schema, sq.access_levels) {
            clauses.push((Occur::Must, filter));
        }

        let combined = BooleanQuery::new(clauses);

        let (mut results, total_count) = if is_listing {
            // Listing mode: fetch offset+limit docs, sort by kind then FQN, then slice
            let pre_filter_count = searcher.search(&combined, &Count)?;
            let fetch_count = if sq.scope.is_some() {
                pre_filter_count
            } else {
                sq.offset.saturating_add(sq.limit)
            };
            let (top_docs, _) =
                searcher.search(&combined, &(TopDocs::with_limit(fetch_count), Count))?;

            let mut all_results: Vec<SearchResult> = top_docs
                .into_iter()
                .map(|(_score, addr)| {
                    let doc: tantivy::TantivyDocument = searcher.doc(addr).unwrap();
                    doc_to_search_result(&schema, &doc)
                })
                .collect();

            all_results.sort_by(|a, b| {
                let kind_order = |k: &SymbolKind| -> u8 {
                    match k {
                        SymbolKind::Class => 0,
                        SymbolKind::Method => 1,
                        SymbolKind::Field => 2,
                    }
                };
                kind_order(&a.symbol_kind)
                    .cmp(&kind_order(&b.symbol_kind))
                    .then_with(|| a.fqn.cmp(&b.fqn))
            });

            // Apply scope filter before pagination in listing mode
            if let Some(scope_filter) = sq.scope {
                all_results.retain(|r| r.scopes.iter().any(|s| s == scope_filter));
                let total = all_results.len();
                let sliced: Vec<SearchResult> = all_results
                    .into_iter()
                    .skip(sq.offset)
                    .take(sq.limit)
                    .collect();
                return Ok((sliced, total, matched_gavs));
            }

            let sliced: Vec<SearchResult> = all_results
                .into_iter()
                .skip(sq.offset)
                .take(sq.limit)
                .collect();
            (sliced, pre_filter_count)
        } else {
            // Search mode: let Tantivy handle offset/limit with relevance ranking
            let (top_docs, total_count) = searcher.search(
                &combined,
                &(TopDocs::with_limit(sq.limit).and_offset(sq.offset), Count),
            )?;
            let results: Vec<SearchResult> = top_docs
                .into_iter()
                .map(|(_score, addr)| {
                    let doc: tantivy::TantivyDocument = searcher.doc(addr).unwrap();
                    doc_to_search_result(&schema, &doc)
                })
                .collect();
            (results, total_count)
        };

        // Scope filter (post-query refinement, search mode only)
        if let Some(scope_filter) = sq.scope {
            results.retain(|r| r.scopes.iter().any(|s| s == scope_filter));
        }

        Ok((results, total_count, matched_gavs))
    }

    /// Build a GAV filter query from a dependency pattern.
    ///
    /// Returns a filter with matched GAVs, or a `None` query when a glob
    /// pattern matches zero GAVs (caller should short-circuit with no results).
    fn build_gav_filter(&self, schema: &Schema, dep: &str) -> Result<GavFilter> {
        let gav_field = schema.get_field("gav").unwrap();
        if dep.contains('*') {
            let all_gavs = self.list_gavs()?;
            let matching: Vec<String> = all_gavs
                .iter()
                .filter(|(g, _)| matches_gav_pattern(g, dep))
                .map(|(g, _)| g.clone())
                .collect();
            if matching.is_empty() {
                return Ok((None, vec![]));
            }
            let gav_clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = matching
                .iter()
                .map(|g| {
                    (
                        Occur::Should,
                        Box::new(TermQuery::new(
                            tantivy::Term::from_field_text(gav_field, g),
                            IndexRecordOption::Basic,
                        )) as Box<dyn tantivy::query::Query>,
                    )
                })
                .collect();
            Ok((Some(Box::new(BooleanQuery::new(gav_clauses))), matching))
        } else {
            Ok((
                Some(Box::new(TermQuery::new(
                    tantivy::Term::from_field_text(gav_field, dep),
                    IndexRecordOption::Basic,
                ))),
                vec![dep.to_string()],
            ))
        }
    }

    /// Return the total number of indexed symbol documents.
    pub fn count_symbols(&self) -> Result<usize> {
        let searcher = self.reader.searcher();
        let count = searcher.search(&AllQuery, &tantivy::collector::Count)?;
        Ok(count)
    }

    /// List all unique GAVs in the index with their symbol counts.
    ///
    /// Iterates the term dictionary of the `gav` field across all segments,
    /// deduplicating via a `BTreeMap`. Returns GAV-sorted `(gav, count)` pairs.
    pub fn list_gavs(&self) -> Result<Vec<(String, usize)>> {
        let schema = self.index.schema();
        let searcher = self.reader.searcher();
        let gav_field = schema.get_field("gav").unwrap();

        // Collect unique GAVs from the term dictionary
        let mut gav_set: BTreeMap<String, ()> = BTreeMap::new();
        for segment_reader in searcher.segment_readers() {
            let inverted_index = segment_reader.inverted_index(gav_field)?;
            let mut term_stream = inverted_index.terms().stream()?;
            while term_stream.advance() {
                let term_bytes = term_stream.key();
                if let Ok(term_str) = std::str::from_utf8(term_bytes) {
                    gav_set.entry(term_str.to_string()).or_insert(());
                }
            }
        }

        // Count symbols per GAV
        let mut results = Vec::with_capacity(gav_set.len());
        for gav in gav_set.into_keys() {
            let term = tantivy::Term::from_field_text(gav_field, &gav);
            let query = TermQuery::new(term, IndexRecordOption::Basic);
            let count = searcher.search(&query, &Count)?;
            results.push((gav, count));
        }

        Ok(results)
    }
}

/// Classify the search mode and build the base Tantivy query.
fn build_base_query(
    index: &Index,
    schema: &Schema,
    sq: &SearchQuery,
) -> Result<Box<dyn tantivy::query::Query>> {
    let Some(query_str) = sq.query else {
        return Ok(Box::new(AllQuery));
    };

    if sq.fqn_mode {
        let fqn_field = schema.get_field("fqn").unwrap();
        return Ok(Box::new(TermQuery::new(
            tantivy::Term::from_field_text(fqn_field, query_str),
            IndexRecordOption::Basic,
        )));
    }

    if sq.regex_mode {
        let simple_name_field = schema.get_field("simple_name").unwrap();
        return Ok(Box::new(RegexQuery::from_pattern(
            query_str,
            simple_name_field,
        )?));
    }

    // Auto-detect FQN (2+ dots)
    if query_str.chars().filter(|&c| c == '.').count() >= 2 {
        let fqn_field = schema.get_field("fqn").unwrap();
        return Ok(Box::new(TermQuery::new(
            tantivy::Term::from_field_text(fqn_field, query_str),
            IndexRecordOption::Basic,
        )));
    }

    // Smart token search with prefix matching
    let simple_name = schema.get_field("simple_name").unwrap();
    let name_parts = schema.get_field("name_parts").unwrap();
    let mut parser = QueryParser::for_index(index, vec![simple_name, name_parts]);
    parser.set_conjunction_by_default();
    let token_query = parser.parse_query(query_str)?;

    // Build prefix queries so that e.g. "murmur" matches "murmur3".
    // Each query word must prefix-match in at least one search field.
    let words: Vec<&str> = query_str.split_whitespace().collect();
    let mut per_word: Vec<(Occur, Box<dyn tantivy::query::Query>)> = Vec::new();
    for word in &words {
        let escaped = regex_escape_term(&word.to_lowercase());
        let pattern = format!("{escaped}.*");
        let mut field_clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = Vec::new();
        if let Ok(q) = RegexQuery::from_pattern(&pattern, simple_name) {
            field_clauses.push((Occur::Should, Box::new(q)));
        }
        if let Ok(q) = RegexQuery::from_pattern(&pattern, name_parts) {
            field_clauses.push((Occur::Should, Box::new(q)));
        }
        if !field_clauses.is_empty() {
            per_word.push((Occur::Must, Box::new(BooleanQuery::new(field_clauses))));
        }
    }

    // Combine: exact token match OR prefix match
    Ok(Box::new(BooleanQuery::new(vec![
        (Occur::Should, token_query),
        (Occur::Should, Box::new(BooleanQuery::new(per_word))),
    ])))
}

/// Build a symbol type filter from a comma-separated kinds string.
///
/// Returns `None` when `symbol_type` is `"any"` (no filtering needed).
fn build_symbol_type_filter(
    schema: &Schema,
    symbol_type: &str,
) -> Option<Box<dyn tantivy::query::Query>> {
    if symbol_type == "any" {
        return None;
    }

    let kind_field = schema.get_field("symbol_kind").unwrap();
    let types: Vec<&str> = symbol_type.split(',').map(|s| s.trim()).collect();

    if types.len() == 1 {
        Some(Box::new(TermQuery::new(
            tantivy::Term::from_field_text(kind_field, types[0]),
            IndexRecordOption::Basic,
        )))
    } else {
        let type_clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = types
            .iter()
            .map(|&t| {
                (
                    Occur::Should,
                    Box::new(TermQuery::new(
                        tantivy::Term::from_field_text(kind_field, t),
                        IndexRecordOption::Basic,
                    )) as Box<dyn tantivy::query::Query>,
                )
            })
            .collect();
        Some(Box::new(BooleanQuery::new(type_clauses)))
    }
}

/// Build an access level filter from a list of allowed levels.
///
/// Returns `None` when `access_levels` is `None` (no filtering needed).
fn build_access_level_filter(
    schema: &Schema,
    access_levels: Option<&[&str]>,
) -> Option<Box<dyn tantivy::query::Query>> {
    let levels = access_levels?;
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
    Some(Box::new(BooleanQuery::new(level_clauses)))
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

    let scopes_str = get_text("scopes");
    let scopes: Vec<String> = if scopes_str.is_empty() {
        vec![]
    } else {
        scopes_str.split(',').map(|s| s.to_string()).collect()
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
        scopes,
    }
}

/// Escape regex special characters in a search term for use in [`RegexQuery`].
fn regex_escape_term(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len() * 2);
    for c in term.chars() {
        if matches!(
            c,
            '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' | '\\'
        ) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}
