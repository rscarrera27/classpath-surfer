//! Handler for the `deps` subcommand.
//!
//! Lists indexed dependencies (GAV coordinates) with their symbol counts.

use std::path::Path;

use anyhow::Result;

use crate::index::reader::IndexReader;
use crate::model::{DepInfo, DepsOutput};

/// List indexed dependencies, optionally filtered by a glob pattern and/or classpath.
pub fn run(
    project_dir: &Path,
    query: Option<&str>,
    classpath: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<DepsOutput> {
    super::require_index(project_dir)?;

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir)?;
    let all_gavs = reader.list_gavs()?;

    // Load manifest for classpath info
    let manifest_path = project_dir.join(".classpath-surfer/classpath-manifest.json");
    let classpath_map = if manifest_path.exists() {
        let content = std::fs::read_to_string(&manifest_path)?;
        let manifest: crate::manifest::ClasspathManifest = serde_json::from_str(&content)?;
        manifest.classpaths_by_gav()
    } else {
        std::collections::HashMap::new()
    };

    let filtered: Vec<&(String, usize)> = if let Some(pattern) = query {
        all_gavs
            .iter()
            .filter(|(gav, _)| super::matches_glob_pattern(gav, pattern))
            .collect()
    } else {
        all_gavs.iter().collect()
    };

    // Apply classpath filter
    let filtered: Vec<&(String, usize)> = if let Some(classpath_filter) = classpath {
        filtered
            .into_iter()
            .filter(|(gav, _)| {
                classpath_map
                    .get(gav.as_str())
                    .is_some_and(|classpaths| classpaths.contains(classpath_filter))
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
            let classpaths: Vec<String> = classpath_map
                .get(gav.as_str())
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            DepInfo {
                gav: gav.clone(),
                symbol_count: *count,
                classpaths,
            }
        })
        .collect();

    let has_more = offset + page.len() < total_count;

    Ok(DepsOutput {
        query: query.map(|s| s.to_string()),
        total_count,
        offset,
        limit,
        has_more,
        dependencies: page,
    })
}
