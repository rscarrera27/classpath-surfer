//! Index staleness detection and marker persistence.
//!
//! Determines whether the local symbol index is out-of-date by comparing
//! a stored lockfile hash and build-file modification times against the
//! current project state.  After a successful refresh,
//! [`writer::update_markers`](crate::staleness::writer::update_markers) saves new staleness markers.

/// Build file modification time comparison.
pub mod buildfiles;
/// Gradle lockfile SHA-256 hash comparison.
pub mod lockfile;
/// Staleness marker persistence after a successful refresh.
pub mod writer;

use std::path::Path;

use anyhow::Result;

/// Check if the index is stale relative to the project's dependency files.
/// Returns Ok(true) if stale, Ok(false) if up-to-date.
pub fn is_stale(project_dir: &Path) -> Result<bool> {
    let surfer_dir = project_dir.join(".classpath-surfer");
    if !surfer_dir.join("indexed-manifest.json").exists() {
        // No index at all
        return Ok(true);
    }

    // Try lockfile-based check first
    if let Some(stale) = lockfile::check_lockfile(project_dir)? {
        return Ok(stale);
    }

    // Fallback to build file mtime check
    buildfiles::check_build_files(project_dir)
}
