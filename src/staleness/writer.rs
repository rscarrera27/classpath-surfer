//! Staleness marker persistence after a successful refresh.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use sha2::{Digest, Sha256};

use super::buildfiles::BUILD_FILES;

/// Update all staleness markers for the project.
///
/// Saves the lockfile SHA-256 hash (if `gradle.lockfile` exists) and
/// records the current modification times of all tracked build files.
pub fn update_markers(project_dir: &Path) -> Result<()> {
    let surfer_dir = project_dir.join(".classpath-surfer");

    // Save lockfile hash if lockfile exists
    let lockfile = project_dir.join("gradle.lockfile");
    if lockfile.exists() {
        let content = std::fs::read(&lockfile)?;
        let hash = format!("{:x}", Sha256::digest(&content));
        std::fs::write(surfer_dir.join("lockfile-hash"), &hash)?;
    }

    // Save build file mtimes
    save_build_file_mtimes(project_dir, &surfer_dir)?;

    Ok(())
}

fn save_build_file_mtimes(project_dir: &Path, surfer_dir: &Path) -> Result<()> {
    let mut mtimes: HashMap<String, u64> = HashMap::new();

    // Collect from root
    collect_mtimes(project_dir, project_dir, BUILD_FILES, &mut mtimes);

    // Collect from submodules (detected from settings file)
    if let Ok(modules) = detect_submodule_dirs(project_dir) {
        for module_dir in modules {
            collect_mtimes(
                project_dir,
                &module_dir,
                &["build.gradle", "build.gradle.kts"],
                &mut mtimes,
            );
        }
    }

    let json = serde_json::to_string_pretty(&mtimes)?;
    std::fs::write(surfer_dir.join("build-file-mtimes.json"), json)?;

    Ok(())
}

/// Collect modification times for build files, storing paths relative to `project_dir`.
fn collect_mtimes(
    project_dir: &Path,
    dir: &Path,
    filenames: &[&str],
    mtimes: &mut HashMap<String, u64>,
) {
    for filename in filenames {
        let path = dir.join(filename);
        if let Ok(meta) = std::fs::metadata(&path)
            && let Ok(modified) = meta.modified()
            && let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            // Store relative path so the project remains portable across symlinks
            // and directory moves (e.g. macOS /var → /private/var).
            let rel = path
                .strip_prefix(project_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            mtimes.insert(rel, duration.as_secs());
        }
    }
}

/// Simple heuristic to detect submodule directories from settings file.
fn detect_submodule_dirs(project_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();

    for settings_file in ["settings.gradle", "settings.gradle.kts"] {
        let path = project_dir.join(settings_file);
        if let Ok(content) = std::fs::read_to_string(&path) {
            // Match patterns like include ':app', include(":lib:core")
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with("include") {
                    // Extract module paths from include statements
                    for part in line.split(['\'', '"']) {
                        let part = part.trim();
                        if let Some(stripped) = part.strip_prefix(':') {
                            let module_path = stripped.replace(':', "/");
                            let module_dir = project_dir.join(&module_path);
                            if module_dir.is_dir() {
                                dirs.push(module_dir);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(dirs)
}
