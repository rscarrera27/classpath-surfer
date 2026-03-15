//! Handler for the `list` subcommand.
//!
//! Lists all symbols for dependencies matching a GAV pattern.

use std::path::Path;

use anyhow::Result;

use crate::index::reader::IndexReader;
use crate::model::ListOutput;

/// List symbols from dependencies matching `gav_pattern`.
pub fn run(
    project_dir: &Path,
    gav_pattern: &str,
    symbol_types: &[&str],
    access_levels: Option<&[&str]>,
    limit: usize,
    offset: usize,
) -> Result<ListOutput> {
    super::require_index(project_dir)?;

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir)?;
    let all_gavs = reader.list_gavs()?;

    let matched_gavs: Vec<String> = all_gavs
        .iter()
        .filter(|(gav, _)| super::matches_gav_pattern(gav, gav_pattern))
        .map(|(gav, _)| gav.clone())
        .collect();

    if matched_gavs.is_empty() {
        return Ok(ListOutput {
            gav_pattern: gav_pattern.to_string(),
            matched_gavs: vec![],
            total_symbols: 0,
            offset,
            limit,
            has_more: false,
            symbols: vec![],
        });
    }

    let (symbols, total_symbols) =
        reader.list_symbols(&matched_gavs, symbol_types, access_levels, limit, offset)?;

    let has_more = offset + symbols.len() < total_symbols;

    Ok(ListOutput {
        gav_pattern: gav_pattern.to_string(),
        matched_gavs,
        total_symbols,
        offset,
        limit,
        has_more,
        symbols,
    })
}
