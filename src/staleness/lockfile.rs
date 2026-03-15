use std::path::Path;

use anyhow::Result;
use sha2::{Digest, Sha256};

/// Check staleness via gradle.lockfile hash.
/// Returns None if no lockfile exists (caller should use fallback).
/// Returns Some(true) if stale, Some(false) if up-to-date.
pub fn check_lockfile(project_dir: &Path) -> Result<Option<bool>> {
    let lockfile = project_dir.join("gradle.lockfile");
    if !lockfile.exists() {
        return Ok(None);
    }

    let saved_hash_path = project_dir.join(".classpath-surfer/lockfile-hash");
    if !saved_hash_path.exists() {
        // No saved hash — treat as stale
        return Ok(Some(true));
    }

    let current_content = std::fs::read(&lockfile)?;
    let current_hash = format!("{:x}", Sha256::digest(&current_content));
    let saved_hash = std::fs::read_to_string(&saved_hash_path)?
        .trim()
        .to_string();

    Ok(Some(current_hash != saved_hash))
}
