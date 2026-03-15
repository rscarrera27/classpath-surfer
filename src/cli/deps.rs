//! Handler for the `deps` subcommand.
//!
//! Lists indexed dependencies (GAV coordinates) with their symbol counts.

use std::path::Path;

use anyhow::Result;

use crate::index::reader::IndexReader;
use crate::model::{DepInfo, DepsOutput};

/// List indexed dependencies, optionally filtered by a glob pattern and/or scope.
pub fn run(
    project_dir: &Path,
    filter: Option<&str>,
    scope: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<DepsOutput> {
    super::require_index(project_dir)?;

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir)?;
    let all_gavs = reader.list_gavs()?;

    // Load manifest for scope info
    let manifest_path = project_dir.join(".classpath-surfer/classpath-manifest.json");
    let scope_map = if manifest_path.exists() {
        let content = std::fs::read_to_string(&manifest_path)?;
        let manifest: crate::manifest::ClasspathManifest = serde_json::from_str(&content)?;
        manifest.scopes_by_gav()
    } else {
        std::collections::HashMap::new()
    };

    let filtered: Vec<&(String, usize)> = if let Some(pattern) = filter {
        all_gavs
            .iter()
            .filter(|(gav, _)| super::matches_gav_pattern(gav, pattern))
            .collect()
    } else {
        all_gavs.iter().collect()
    };

    // Apply scope filter
    let filtered: Vec<&(String, usize)> = if let Some(scope_filter) = scope {
        filtered
            .into_iter()
            .filter(|(gav, _)| {
                scope_map
                    .get(gav.as_str())
                    .is_some_and(|scopes| scopes.contains(scope_filter))
            })
            .collect()
    } else {
        filtered
    };

    let total_count = filtered.len();
    let page: Vec<DepInfo> = filtered
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(gav, count)| {
            let scopes: Vec<String> = scope_map
                .get(gav.as_str())
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            DepInfo {
                gav: gav.clone(),
                symbol_count: *count,
                scopes,
            }
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
