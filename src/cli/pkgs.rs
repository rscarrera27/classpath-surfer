//! Handler for the `pkgs` subcommand.
//!
//! Lists unique Java packages in the index with their symbol counts.

use std::path::Path;

use anyhow::Result;

use crate::index::reader::IndexReader;
use crate::model::{PkgInfo, PkgsOutput, matches_gav_pattern};

/// List indexed packages, optionally filtered by a glob pattern and/or dependency.
pub fn run(
    project_dir: &Path,
    filter: Option<&str>,
    dependency: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<PkgsOutput> {
    super::require_index(project_dir)?;

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir)?;

    let (all_pkgs, matched_gavs) = if let Some(dep) = dependency {
        let (pkgs, gavs) = reader.list_packages_for_dependency(dep)?;
        (pkgs, Some(gavs))
    } else {
        (reader.list_packages()?, None)
    };

    let filtered: Vec<&(String, usize)> = if let Some(pattern) = filter {
        all_pkgs
            .iter()
            .filter(|(pkg, _)| matches_gav_pattern(pkg, pattern))
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
        filter: filter.map(|s| s.to_string()),
        dependency: dependency.map(|s| s.to_string()),
        matched_gavs,
        total_count,
        offset,
        limit,
        has_more,
        packages: page,
    })
}
