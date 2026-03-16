//! Handler for the `pkgs` subcommand.
//!
//! Lists unique Java packages in the index with their symbol counts.

use std::path::Path;

use anyhow::Result;

use crate::index::reader::IndexReader;
use crate::model::{PkgInfo, PkgsOutput, matches_gav_pattern};

/// List indexed packages, optionally filtered by a glob pattern.
pub fn run(
    project_dir: &Path,
    filter: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<PkgsOutput> {
    super::require_index(project_dir)?;

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir)?;
    let all_pkgs = reader.list_packages()?;

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
        total_count,
        offset,
        limit,
        has_more,
        packages: page,
    })
}
