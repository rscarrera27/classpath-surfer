//! CLI subcommand dispatch.
//!
//! Each subcommand (`init`, `refresh`, `search`, `show`, `status`, `clean`) lives
//! in its own module and is wired into the top-level clap `Commands` enum.
//! The `render` module provides plain-text renderers for non-TTY output.

use std::path::Path;

use anyhow::Result;

use crate::error::CliError;
use crate::staleness;

/// Remove index data and staleness markers.
pub mod clean;
/// Project initialization (config, Gradle init script).
pub mod init;
/// Classpath extraction, manifest merge, and index build/update.
pub mod refresh;
/// Plain-text renderers for non-TTY output.
pub mod render;
/// Symbol search with text, FQN, and regex modes.
pub mod search;
/// Source code display from source JARs or decompilation.
pub mod show;
/// Index status reporting (dependency count, symbol count, staleness, disk size).
pub mod status;

/// Verify that the Tantivy index exists and is not stale.
///
/// Returns `Ok(())` if `meta.json` is present and the index is fresh,
/// or an appropriate [`CliError`] otherwise.
pub fn require_index(project_dir: &Path) -> Result<()> {
    let index_dir = project_dir.join(".classpath-surfer/index");
    if !index_dir.join("meta.json").exists() {
        return Err(CliError::resource_not_found(
            "INDEX_NOT_FOUND",
            "No index found. Run `classpath-surfer refresh` to build it.",
        )
        .with_suggested_command("classpath-surfer refresh")
        .into());
    }
    check_staleness(project_dir)
}

/// Verify that the classpath manifest exists and the index is not stale.
///
/// Returns `Ok(())` if `classpath-manifest.json` is present and the index
/// is fresh, or an appropriate [`CliError`] otherwise.
pub fn require_manifest(project_dir: &Path) -> Result<()> {
    let manifest_path = project_dir.join(".classpath-surfer/classpath-manifest.json");
    if !manifest_path.exists() {
        return Err(CliError::resource_not_found(
            "INDEX_NOT_FOUND",
            "No index found. Run `classpath-surfer refresh` to build it.",
        )
        .with_suggested_command("classpath-surfer refresh")
        .into());
    }
    check_staleness(project_dir)
}

fn check_staleness(project_dir: &Path) -> Result<()> {
    if staleness::is_stale(project_dir)? {
        return Err(CliError::resource_not_found(
            "INDEX_STALE",
            "Index is stale. Dependencies have changed since last indexing.\n\
             Run `classpath-surfer refresh` to update.",
        )
        .with_suggested_command("classpath-surfer refresh")
        .into());
    }
    Ok(())
}
