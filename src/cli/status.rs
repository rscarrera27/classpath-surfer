use std::path::Path;

use anyhow::Result;

use crate::error::CliError;
use crate::index::reader::IndexReader;
use crate::manifest::ClasspathManifest;
use crate::model::StatusOutput;
use crate::staleness;

/// Collect index status and return it as structured data.
pub fn run(project_dir: &Path) -> Result<StatusOutput> {
    let surfer_dir = project_dir.join(".classpath-surfer");

    if !surfer_dir.exists() {
        return Ok(StatusOutput {
            initialized: false,
            has_index: false,
            dependency_count: 0,
            with_source_jars: 0,
            without_source_jars: 0,
            indexed_symbols: None,
            is_stale: false,
            index_size: None,
        });
    }

    let manifest_path = surfer_dir.join("classpath-manifest.json");
    let index_dir = surfer_dir.join("index");

    if !manifest_path.exists() {
        return Ok(StatusOutput {
            initialized: true,
            has_index: false,
            dependency_count: 0,
            with_source_jars: 0,
            without_source_jars: 0,
            indexed_symbols: None,
            is_stale: false,
            index_size: None,
        });
    }

    let manifest_json = std::fs::read_to_string(&manifest_path)?;
    let manifest: ClasspathManifest = serde_json::from_str(&manifest_json)?;
    let deps = manifest.all_dependencies();

    let source_count = deps.iter().filter(|d| d.source_jar_path.is_some()).count();

    let has_index = index_dir.join("meta.json").exists();
    let indexed_symbols = if has_index {
        let reader = IndexReader::open(&index_dir).map_err(|e| {
            CliError::general("INDEX_OPEN_FAILED", format!("Failed to open index: {e}"))
        })?;
        Some(reader.count_symbols()?)
    } else {
        None
    };

    let stale = staleness::is_stale(project_dir)?;

    let index_size = if index_dir.exists() {
        Some(format_size(dir_size(&index_dir)?))
    } else {
        None
    };

    Ok(StatusOutput {
        initialized: true,
        has_index,
        dependency_count: deps.len(),
        with_source_jars: source_count,
        without_source_jars: deps.len() - source_count,
        indexed_symbols,
        is_stale: stale,
        index_size,
    })
}

fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            total += meta.len();
        }
    }
    Ok(total)
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
