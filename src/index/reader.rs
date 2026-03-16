use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, Occur, QueryParser, RegexQuery, TermQuery};
use tantivy::schema::*;
use tantivy::{Index, ReloadPolicy};

use crate::error::CliError;
use crate::model::{
    AccessLevel, SearchQuery, SearchResult, SignatureDisplay, SourceLanguage, SymbolKind,
    glob_to_tantivy_regex, matches_glob_pattern,
};

/// Result of GAV filter construction: the Tantivy query clause (if any) and matched GAVs.
type GavFilter = (Option<Box<dyn tantivy::query::Query>>, Vec<String>);

/// Packages with symbol counts, paired with the GAVs that were matched.
type PackagesWithGavs = (Vec<(String, usize)>, Vec<String>);

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
    /// to be rebuilt via `classpath-surfer index refresh --force`.
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
            "classpaths",
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
                     Run `classpath-surfer index refresh` to rebuild.",
                    missing.join(", ")
                ),
            )
            .with_suggested_command("classpath-surfer index refresh")
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
    /// Search mode is auto-detected from the query string:
    ///
    /// - **Glob on FQN** — glob chars (`*`/`?`) + 2+ dots → regex on `fqn`.
    /// - **Glob on simple name** — glob chars, < 2 dots → regex on `simple_name`.
    /// - **FQN exact** — 2+ dots, no glob chars → `TermQuery` on `fqn`.
    /// - **Smart search** — everything else → token search on `simple_name` +
    ///   `name_parts` with AND semantics and prefix matching.
    ///
    /// When `query` is `None`, all symbols are returned (requires `lib`).
    /// Results without a text query are sorted by kind then FQN.
    ///
    /// Results can be narrowed by symbol type, a `dependency` GAV pattern
    /// (glob with `*`/`?`), access level, package pattern, and classpath.
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
        if let Some(filter) = build_symbol_type_filter(&schema, sq.symbol_types) {
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

        // Package filter
        if let Some(pkg) = sq.package {
            if let Some(filter) = self.build_package_filter(&schema, pkg)? {
                clauses.push((Occur::Must, filter));
            } else {
                // Empty match — no results possible
                return Ok((vec![], 0, matched_gavs));
            }
        }

        let combined = BooleanQuery::new(clauses);

        let (mut results, total_count) = if is_listing {
            // Listing mode: fetch offset+limit docs, sort by kind then FQN, then slice
            let pre_filter_count = searcher.search(&combined, &Count)?;
            let fetch_count = if sq.classpath.is_some() {
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

            // Apply classpath filter before pagination in listing mode
            if let Some(classpath_filter) = sq.classpath {
                all_results.retain(|r| r.classpaths.iter().any(|s| s == classpath_filter));
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

        // Classpath filter (post-query refinement, search mode only)
        if let Some(classpath_filter) = sq.classpath {
            results.retain(|r| r.classpaths.iter().any(|s| s == classpath_filter));
        }

        Ok((results, total_count, matched_gavs))
    }

    /// Build a GAV filter query from a dependency GAV pattern.
    ///
    /// Returns a filter with matched GAVs, or a `None` query when a glob
    /// pattern matches zero GAVs (caller should short-circuit with no results).
    fn build_gav_filter(&self, schema: &Schema, dep: &str) -> Result<GavFilter> {
        let gav_field = schema.get_field("gav").unwrap();
        if dep.contains('*') || dep.contains('?') {
            let all_gavs = self.list_gavs()?;
            let matching: Vec<String> = all_gavs
                .iter()
                .filter(|(g, _)| matches_glob_pattern(g, dep))
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

    /// Build a package filter query from a package pattern.
    ///
    /// Returns `None` when a glob pattern matches zero packages (caller should
    /// short-circuit with no results).
    fn build_package_filter(
        &self,
        schema: &Schema,
        pattern: &str,
    ) -> Result<Option<Box<dyn tantivy::query::Query>>> {
        let pkg_field = schema.get_field("package").unwrap();
        if pattern.contains('*') || pattern.contains('?') {
            let searcher = self.reader.searcher();
            let mut pkg_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for segment_reader in searcher.segment_readers() {
                let inverted_index = segment_reader.inverted_index(pkg_field)?;
                let mut term_stream = inverted_index.terms().stream()?;
                while term_stream.advance() {
                    let term_bytes = term_stream.key();
                    if let Ok(term_str) = std::str::from_utf8(term_bytes)
                        && matches_glob_pattern(term_str, pattern)
                    {
                        pkg_set.insert(term_str.to_string());
                    }
                }
            }
            if pkg_set.is_empty() {
                return Ok(None);
            }
            let pkg_clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = pkg_set
                .into_iter()
                .map(|p| {
                    (
                        Occur::Should,
                        Box::new(TermQuery::new(
                            tantivy::Term::from_field_text(pkg_field, &p),
                            IndexRecordOption::Basic,
                        )) as Box<dyn tantivy::query::Query>,
                    )
                })
                .collect();
            Ok(Some(Box::new(BooleanQuery::new(pkg_clauses))))
        } else {
            Ok(Some(Box::new(TermQuery::new(
                tantivy::Term::from_field_text(pkg_field, pattern),
                IndexRecordOption::Basic,
            ))))
        }
    }

    /// Return the total number of indexed symbol documents.
    pub fn count_symbols(&self) -> Result<usize> {
        let searcher = self.reader.searcher();
        let count = searcher.search(&AllQuery, &tantivy::collector::Count)?;
        Ok(count)
    }

    /// List all unique packages in the index with their symbol counts.
    ///
    /// Iterates the term dictionary of the `package` field across all segments,
    /// deduplicating via a `BTreeMap`. Returns package-sorted `(package, count)` pairs.
    pub fn list_packages(&self) -> Result<Vec<(String, usize)>> {
        let schema = self.index.schema();
        let searcher = self.reader.searcher();
        let pkg_field = schema.get_field("package").unwrap();

        let mut pkg_set: BTreeMap<String, ()> = BTreeMap::new();
        for segment_reader in searcher.segment_readers() {
            let inverted_index = segment_reader.inverted_index(pkg_field)?;
            let mut term_stream = inverted_index.terms().stream()?;
            while term_stream.advance() {
                let term_bytes = term_stream.key();
                if let Ok(term_str) = std::str::from_utf8(term_bytes) {
                    pkg_set.entry(term_str.to_string()).or_insert(());
                }
            }
        }

        let mut results = Vec::with_capacity(pkg_set.len());
        for pkg in pkg_set.into_keys() {
            let term = tantivy::Term::from_field_text(pkg_field, &pkg);
            let query = TermQuery::new(term, IndexRecordOption::Basic);
            let count = searcher.search(&query, &Count)?;
            results.push((pkg, count));
        }

        Ok(results)
    }

    /// List unique packages within dependencies matching a GAV pattern.
    ///
    /// Searches all documents matching the GAV filter and aggregates by package.
    /// Returns `(packages, matched_gavs)`.
    pub fn list_packages_for_dependency(&self, dep: &str) -> Result<PackagesWithGavs> {
        let schema = self.index.schema();
        let searcher = self.reader.searcher();
        let pkg_field = schema.get_field("package").unwrap();

        let (filter, matched_gavs) = self.build_gav_filter(&schema, dep)?;
        let query = match filter {
            Some(q) => q,
            None => return Ok((vec![], vec![])),
        };

        let total = searcher.search(&query, &Count)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(total))?;

        let mut pkg_counts: BTreeMap<String, usize> = BTreeMap::new();
        for (_score, addr) in top_docs {
            let doc: tantivy::TantivyDocument = searcher.doc(addr)?;
            if let Some(pkg_val) = doc.get_first(pkg_field).and_then(|v| v.as_str()) {
                *pkg_counts.entry(pkg_val.to_string()).or_insert(0) += 1;
            }
        }

        let results: Vec<(String, usize)> = pkg_counts.into_iter().collect();
        Ok((results, matched_gavs))
    }

    /// List unique packages for a specific set of GAV strings.
    ///
    /// Builds a single boolean OR query from the GAV list and aggregates
    /// packages with their symbol counts. Returns `(packages, matched_gavs)`.
    pub fn list_packages_for_gavs(&self, gavs: &[&str]) -> Result<PackagesWithGavs> {
        if gavs.is_empty() {
            return Ok((vec![], vec![]));
        }

        let schema = self.index.schema();
        let searcher = self.reader.searcher();
        let gav_field = schema.get_field("gav").unwrap();
        let pkg_field = schema.get_field("package").unwrap();

        let clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = gavs
            .iter()
            .map(|gav| {
                let term = tantivy::Term::from_field_text(gav_field, gav);
                (
                    Occur::Should,
                    Box::new(TermQuery::new(term, IndexRecordOption::Basic))
                        as Box<dyn tantivy::query::Query>,
                )
            })
            .collect();

        let query = BooleanQuery::new(clauses);
        let total = searcher.search(&query, &Count)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(total))?;

        let mut pkg_counts: BTreeMap<String, usize> = BTreeMap::new();
        for (_score, addr) in top_docs {
            let doc: tantivy::TantivyDocument = searcher.doc(addr)?;
            if let Some(pkg_val) = doc.get_first(pkg_field).and_then(|v| v.as_str()) {
                *pkg_counts.entry(pkg_val.to_string()).or_insert(0) += 1;
            }
        }

        let results: Vec<(String, usize)> = pkg_counts.into_iter().collect();
        Ok((results, gavs.iter().map(|s| s.to_string()).collect()))
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
///
/// Search mode classification (auto-detected from query string):
/// 1. **Glob on FQN** — query has glob chars (`*`/`?`) and 2+ dots →
///    `RegexQuery` on `fqn` field (case-sensitive).
/// 2. **Glob on simple name** — query has glob chars but fewer than 2 dots →
///    `RegexQuery` on `simple_name` field (lowercased).
/// 3. **FQN exact match** — query has 2+ dots, no glob chars →
///    `TermQuery` on `fqn` field.
/// 4. **Smart token search** — everything else → token search on
///    `simple_name` + `name_parts` with prefix matching.
fn build_base_query(
    index: &Index,
    schema: &Schema,
    sq: &SearchQuery,
) -> Result<Box<dyn tantivy::query::Query>> {
    let Some(query_str) = sq.query else {
        return Ok(Box::new(AllQuery));
    };

    let has_glob = query_str.contains('*') || query_str.contains('?');
    let dot_count = query_str.chars().filter(|&c| c == '.').count();

    if has_glob && dot_count >= 2 {
        // Glob on FQN (case-sensitive)
        let fqn_field = schema.get_field("fqn").unwrap();
        return Ok(Box::new(RegexQuery::from_pattern(
            &glob_to_tantivy_regex(query_str),
            fqn_field,
        )?));
    }

    if has_glob {
        // Glob on simple_name (lowercase)
        let simple_name_field = schema.get_field("simple_name").unwrap();
        return Ok(Box::new(RegexQuery::from_pattern(
            &glob_to_tantivy_regex(&query_str.to_lowercase()),
            simple_name_field,
        )?));
    }

    // Auto-detect FQN exact match (2+ dots)
    if dot_count >= 2 {
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

/// Build a symbol type filter from a slice of [`SymbolKind`] values.
///
/// Returns `None` when `symbol_types` is empty (no filtering needed).
fn build_symbol_type_filter(
    schema: &Schema,
    symbol_types: &[SymbolKind],
) -> Option<Box<dyn tantivy::query::Query>> {
    if symbol_types.is_empty() {
        return None;
    }

    let kind_field = schema.get_field("symbol_kind").unwrap();

    if symbol_types.len() == 1 {
        Some(Box::new(TermQuery::new(
            tantivy::Term::from_field_text(kind_field, symbol_types[0].as_str()),
            IndexRecordOption::Basic,
        )))
    } else {
        let type_clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = symbol_types
            .iter()
            .map(|t| {
                (
                    Occur::Should,
                    Box::new(TermQuery::new(
                        tantivy::Term::from_field_text(kind_field, t.as_str()),
                        IndexRecordOption::Basic,
                    )) as Box<dyn tantivy::query::Query>,
                )
            })
            .collect();
        Some(Box::new(BooleanQuery::new(type_clauses)))
    }
}

/// Build an access level filter from a slice of [`AccessLevel`] values.
///
/// Returns `None` when `access_levels` is empty or contains [`AccessLevel::All`].
fn build_access_level_filter(
    schema: &Schema,
    access_levels: &[AccessLevel],
) -> Option<Box<dyn tantivy::query::Query>> {
    // Collect only concrete levels (skip All)
    let terms: Vec<&str> = access_levels
        .iter()
        .filter_map(|l| l.as_index_str())
        .collect();

    if terms.is_empty() {
        return None;
    }

    let al_field = schema.get_field("access_level").unwrap();
    let level_clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = terms
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

    let classpaths_str = get_text("classpaths");
    let classpaths: Vec<String> = if classpaths_str.is_empty() {
        vec![]
    } else {
        classpaths_str.split(',').map(|s| s.to_string()).collect()
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
        classpaths,
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
