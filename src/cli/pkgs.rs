//! Handler for the `pkgs` subcommand.
//!
//! Lists unique Java packages in the index with their symbol counts.

use std::path::Path;

use anyhow::Result;

use crate::index::reader::IndexReader;
use crate::model::{PkgInfo, PkgsOutput, matches_glob_pattern};

/// List indexed packages, optionally filtered by a glob pattern, dependency, scope, or both.
pub fn run(
    project_dir: &Path,
    query: Option<&str>,
    dependency: Option<&str>,
    scope: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<PkgsOutput> {
    super::require_index(project_dir)?;

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir)?;

    // Resolve scope to a set of GAVs from the manifest
    let scope_gavs = load_scope_gavs(project_dir, scope)?;

    let (all_pkgs, matched_gavs) = match (dependency, &scope_gavs) {
        (Some(dep), Some(gavs)) => {
            // Both: get dependency-matching GAVs, intersect with scope GAVs
            let (_, dep_gavs) = reader.list_packages_for_dependency(dep)?;
            let intersected: Vec<&str> = dep_gavs
                .iter()
                .filter(|g| gavs.contains(g))
                .map(|s| s.as_str())
                .collect();
            let (pkgs, matched) = reader.list_packages_for_gavs(&intersected)?;
            (pkgs, Some(matched))
        }
        (Some(dep), None) => {
            let (pkgs, gavs) = reader.list_packages_for_dependency(dep)?;
            (pkgs, Some(gavs))
        }
        (None, Some(gavs)) => {
            let gav_refs: Vec<&str> = gavs.iter().map(|s| s.as_str()).collect();
            let (pkgs, matched) = reader.list_packages_for_gavs(&gav_refs)?;
            (pkgs, Some(matched))
        }
        (None, None) => (reader.list_packages()?, None),
    };

    let filtered: Vec<&(String, usize)> = if let Some(pattern) = query {
        all_pkgs
            .iter()
            .filter(|(pkg, _)| matches_glob_pattern(pkg, pattern))
            .collect()
    } else {
        all_pkgs.iter().collect()
    };

    let total_count = filtered.len();
    let page: Vec<PkgInfo> = filtered
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(pkg, count)| PkgInfo {
            package: pkg.clone(),
            symbol_count: *count,
        })
        .collect();

    let has_more = offset + page.len() < total_count;

    Ok(PkgsOutput {
        query: query.map(|s| s.to_string()),
        dependency: dependency.map(|s| s.to_string()),
        scope: scope.map(|s| s.to_string()),
        matched_gavs,
        total_count,
        offset,
        limit,
        has_more,
        packages: page,
    })
}

/// Load GAVs belonging to a configuration scope from the classpath manifest.
///
/// Returns `None` if no scope filter is requested or the manifest doesn't exist.
fn load_scope_gavs(project_dir: &Path, scope: Option<&str>) -> Result<Option<Vec<String>>> {
    let Some(scope_filter) = scope else {
        return Ok(None);
    };

    let manifest_path = project_dir.join(".classpath-surfer/classpath-manifest.json");
    if !manifest_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&manifest_path)?;
    let manifest: crate::manifest::ClasspathManifest = serde_json::from_str(&content)?;
    let scope_map = manifest.scopes_by_gav();

    let gavs: Vec<String> = scope_map
        .into_iter()
        .filter(|(_, scopes)| scopes.contains(scope_filter))
        .map(|(gav, _)| gav)
        .collect();

    Ok(Some(gavs))
}
