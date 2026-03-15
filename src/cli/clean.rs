use std::path::Path;

use anyhow::Result;

use crate::error::CliError;
use crate::model::CleanOutput;

/// Remove the symbol index and staleness markers, preserving config and init script.
pub fn run(project_dir: &Path) -> Result<CleanOutput> {
    let surfer_dir = project_dir.join(".classpath-surfer");
    let mut items_removed = Vec::new();

    let index_dir = surfer_dir.join("index");
    if index_dir.exists() {
        std::fs::remove_dir_all(&index_dir).map_err(|e| {
            CliError::general(
                "CLEAN_FAILED",
                format!("Failed to remove index directory: {e}"),
            )
        })?;
        eprintln!("Removed index directory");
        items_removed.push("index directory".to_string());
    }

    let indexed_manifest = surfer_dir.join("indexed-manifest.json");
    if indexed_manifest.exists() {
        std::fs::remove_file(&indexed_manifest).map_err(|e| {
            CliError::general(
                "CLEAN_FAILED",
                format!("Failed to remove indexed manifest: {e}"),
            )
        })?;
        eprintln!("Removed indexed manifest");
        items_removed.push("indexed manifest".to_string());
    }

    let lockfile_hash = surfer_dir.join("lockfile-hash");
    if lockfile_hash.exists() {
        std::fs::remove_file(&lockfile_hash)?;
        items_removed.push("lockfile hash".to_string());
    }

    let mtimes = surfer_dir.join("build-file-mtimes.json");
    if mtimes.exists() {
        std::fs::remove_file(&mtimes)?;
        items_removed.push("build file mtimes".to_string());
    }

    eprintln!("Clean complete. Run `classpath-surfer refresh` to rebuild.");
    Ok(CleanOutput { items_removed })
}
