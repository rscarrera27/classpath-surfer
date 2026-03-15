use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

/// Gradle build file names tracked for staleness detection.
///
/// Shared between the reader ([`check_build_files`]) and the writer
/// ([`super::writer::update_markers`]) so that both sides agree on the
/// set of files to monitor.
pub const BUILD_FILES: &[&str] = &[
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
    "gradle/libs.versions.toml",
];

/// Check staleness via build file modification times.
/// Returns true if stale, false if up-to-date.
pub fn check_build_files(project_dir: &Path) -> Result<bool> {
    let mtimes_path = project_dir.join(".classpath-surfer/build-file-mtimes.json");
    if !mtimes_path.exists() {
        return Ok(true); // No saved mtimes — stale
    }

    let saved_json = std::fs::read_to_string(&mtimes_path)?;
    let saved_mtimes: HashMap<String, u64> = serde_json::from_str(&saved_json)?;

    // Check each saved file's current mtime.
    // Paths are stored relative to project_dir.
    for (rel_path, saved_mtime) in &saved_mtimes {
        let path = project_dir.join(rel_path);
        if !path.exists() {
            // File was deleted — stale
            return Ok(true);
        }
        if let Ok(meta) = std::fs::metadata(&path)
            && let Ok(modified) = meta.modified()
            && let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH)
            && duration.as_secs() != *saved_mtime
        {
            return Ok(true);
        }
    }

    // Also check if new build files appeared that weren't tracked
    for filename in BUILD_FILES {
        let path = project_dir.join(filename);
        if path.exists() && !saved_mtimes.contains_key(*filename) {
            return Ok(true);
        }
    }

    Ok(false)
}
