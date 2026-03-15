use std::path::Path;

use anyhow::Result;

use crate::index::reader::IndexReader;
use crate::model::{SearchOutput, SearchQuery};

/// Search the symbol index and return structured results.
///
/// Supports text, FQN exact-match, and regex search modes, with optional
/// symbol-type and dependency filters.
pub fn run(project_dir: &Path, query: &SearchQuery) -> Result<SearchOutput> {
    super::require_index(project_dir)?;

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir)?;
    let (results, total_matches) = reader.search(query)?;

    Ok(SearchOutput {
        query: query.query.to_string(),
        total_matches,
        results,
    })
}
