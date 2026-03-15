//! Handler for the `deps` subcommand.
//!
//! Lists indexed dependencies (GAV coordinates) with their symbol counts.

use std::path::Path;

use anyhow::Result;

use crate::index::reader::IndexReader;
use crate::model::{DepInfo, DepsOutput};

/// List indexed dependencies, optionally filtered by a glob pattern.
pub fn run(
    project_dir: &Path,
    filter: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<DepsOutput> {
    super::require_index(project_dir)?;

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir)?;
    let all_gavs = reader.list_gavs()?;

    let filtered: Vec<&(String, usize)> = if let Some(pattern) = filter {
        all_gavs
            .iter()
            .filter(|(gav, _)| super::matches_gav_pattern(gav, pattern))
            .collect()
    } else {
        all_gavs.iter().collect()
    };

    let total_count = filtered.len();
    let page: Vec<DepInfo> = filtered
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(gav, count)| DepInfo {
            gav: gav.clone(),
            symbol_count: *count,
        })
        .collect();

    let has_more = offset + page.len() < total_count;

    Ok(DepsOutput {
        filter: filter.map(|s| s.to_string()),
        total_count,
        offset,
        limit,
        has_more,
        dependencies: page,
    })
}
