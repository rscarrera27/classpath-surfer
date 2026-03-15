use std::path::Path;

use anyhow::Result;

use crate::error::CliError;
use crate::gradle::{init_script, runner};
use crate::index::writer;
use crate::manifest::{self, diff, merge};
use crate::model::RefreshOutput;
use crate::staleness;

/// Refresh the symbol index for the project.
///
/// Runs the Gradle `classpathSurferExport` task to extract the classpath,
/// merges per-module manifests, computes a GAV-level diff for incremental
/// indexing (or performs a full rebuild when `force` is true), and updates
/// staleness markers (lockfile hash, build-file mtimes).
///
/// When `force` is false and the index is not stale, the Gradle invocation
/// is skipped entirely and an `up_to_date` result is returned immediately.
pub fn run(project_dir: &Path, configurations: &[String], force: bool) -> Result<RefreshOutput> {
    run_with_java_home(project_dir, configurations, force, None)
}

/// Same as [`run`], but allows overriding `JAVA_HOME` for the Gradle invocation.
pub fn run_with_java_home(
    project_dir: &Path,
    configurations: &[String],
    force: bool,
    java_home: Option<&Path>,
) -> Result<RefreshOutput> {
    let surfer_dir = project_dir.join(".classpath-surfer");
    std::fs::create_dir_all(&surfer_dir)?;

    // 0. Early return: skip Gradle if index is fresh (unless --force)
    if !force {
        let indexed_manifest_path = surfer_dir.join("indexed-manifest.json");
        if indexed_manifest_path.exists() && !staleness::is_stale(project_dir)? {
            eprintln!("Index is up to date. Skipping Gradle invocation.");
            return Ok(RefreshOutput {
                mode: "up_to_date".to_string(),
                dependencies_processed: 0,
                symbols_indexed: 0,
            });
        }
    }

    // 1. Write init script to temp location and run Gradle
    let init_script_path = surfer_dir.join("init-script.gradle");
    std::fs::write(&init_script_path, init_script::INIT_SCRIPT).map_err(|e| {
        CliError::general(
            "INIT_SCRIPT_FAILED",
            format!("Failed to write init script: {e}"),
        )
    })?;

    eprintln!("Running Gradle to extract classpath...");
    runner::run_export_task(project_dir, &init_script_path, configurations, java_home)?;

    // 2. Merge per-module manifests
    let build_dir = project_dir.join("build");
    let current_manifest = merge::merge_module_manifests(&build_dir)?;

    // Save the current manifest
    let manifest_path = surfer_dir.join("classpath-manifest.json");
    let manifest_json = serde_json::to_string_pretty(&current_manifest)?;
    std::fs::write(&manifest_path, &manifest_json)?;

    // 3. Compute diff with previous indexed manifest
    let indexed_manifest_path = surfer_dir.join("indexed-manifest.json");
    let index_dir = surfer_dir.join("index");

    let open_result = writer::open_or_create_index(&index_dir)?;
    let fields = writer::SchemaFields::new(&open_result.index.schema());
    let mut index_writer = writer::create_writer(&open_result.index)?;
    let force_full = force || open_result.schema_rebuilt;

    let unique_deps = merge::deduplicate(&current_manifest);
    let scope_map = current_manifest.scopes_by_gav();

    let output = if force_full || !indexed_manifest_path.exists() {
        // Full index: index everything
        eprintln!(
            "Building full index for {} dependencies...",
            unique_deps.len()
        );
        // Clear existing index
        index_writer.delete_all_documents()?;
        index_writer.commit()?;

        let mut total_symbols = 0;
        for dep in &unique_deps {
            if !dep.jar_path.exists() {
                eprintln!("  warning: JAR not found: {}", dep.jar_path.display());
                continue;
            }
            let scopes_str = scope_map
                .get(&dep.gav())
                .map(|s| s.iter().cloned().collect::<Vec<_>>().join(","))
                .unwrap_or_default();
            match writer::index_dependency(&index_writer, &fields, dep, &scopes_str) {
                Ok(count) => {
                    total_symbols += count;
                    eprintln!("  indexed {} ({} symbols)", dep.gav(), count);
                }
                Err(e) => {
                    eprintln!("  warning: failed to index {}: {e}", dep.gav());
                }
            }
        }

        index_writer.commit()?;
        eprintln!(
            "Indexed {total_symbols} symbols from {} dependencies.",
            unique_deps.len()
        );

        RefreshOutput {
            mode: "full".to_string(),
            dependencies_processed: unique_deps.len(),
            symbols_indexed: total_symbols,
        }
    } else {
        // Incremental index
        let prev_json = std::fs::read_to_string(&indexed_manifest_path)?;
        let prev_manifest: manifest::ClasspathManifest = serde_json::from_str(&prev_json)?;
        let manifest_diff = diff::compute_diff(&current_manifest, &prev_manifest);

        if manifest_diff.added.is_empty() && manifest_diff.removed.is_empty() {
            eprintln!("No dependency changes detected. Index is up to date.");
            // Still update the staleness markers
            staleness::writer::update_markers(project_dir)?;
            return Ok(RefreshOutput {
                mode: "up_to_date".to_string(),
                dependencies_processed: 0,
                symbols_indexed: 0,
            });
        }

        eprintln!(
            "Incremental update: +{} added, -{} removed, {} unchanged",
            manifest_diff.added.len(),
            manifest_diff.removed.len(),
            manifest_diff.unchanged.len()
        );

        // Remove stale GAVs
        for gav in &manifest_diff.removed {
            writer::delete_gav(&index_writer, &fields, gav)?;
            eprintln!("  removed {gav}");
        }

        // Index new GAVs
        let mut total_symbols = 0;
        let mut deps_processed = 0;
        for dep in &unique_deps {
            if !manifest_diff.added.contains(&dep.gav()) {
                continue;
            }
            deps_processed += 1;
            if !dep.jar_path.exists() {
                eprintln!("  warning: JAR not found: {}", dep.jar_path.display());
                continue;
            }
            let scopes_str = scope_map
                .get(&dep.gav())
                .map(|s| s.iter().cloned().collect::<Vec<_>>().join(","))
                .unwrap_or_default();
            match writer::index_dependency(&index_writer, &fields, dep, &scopes_str) {
                Ok(count) => {
                    total_symbols += count;
                    eprintln!("  indexed {} ({} symbols)", dep.gav(), count);
                }
                Err(e) => {
                    eprintln!("  warning: failed to index {}: {e}", dep.gav());
                }
            }
        }

        index_writer.commit()?;
        eprintln!("Added {total_symbols} symbols.");

        RefreshOutput {
            mode: "incremental".to_string(),
            dependencies_processed: deps_processed,
            symbols_indexed: total_symbols,
        }
    };

    // 4. Save indexed manifest (atomic: only after successful commit)
    std::fs::write(&indexed_manifest_path, &manifest_json)?;

    // 5. Update staleness markers
    staleness::writer::update_markers(project_dir)?;

    eprintln!("Done.");
    Ok(output)
}
