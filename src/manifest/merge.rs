use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};

use crate::error::CliError;

use super::{ClasspathManifest, DependencyInfo, ModuleManifest};

/// Read per-module JSON files from `build/classpath-surfer/` and merge into one manifest.
pub fn merge_module_manifests(build_dir: &Path) -> Result<ClasspathManifest> {
    let surfer_dir = build_dir.join("classpath-surfer");
    let mut modules = Vec::new();

    if !surfer_dir.exists() {
        return Err(CliError::resource_not_found(
            "GRADLE_OUTPUT_MISSING",
            format!(
                "No classpath-surfer output found in {}. Did the Gradle task run?",
                surfer_dir.display()
            ),
        )
        .into());
    }

    for entry in std::fs::read_dir(&surfer_dir)
        .with_context(|| format!("reading {}", surfer_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let module: ModuleManifest = serde_json::from_str(&content)
                .with_context(|| format!("parsing {}", path.display()))?;
            modules.push(module);
        }
    }

    if modules.is_empty() {
        return Err(CliError::resource_not_found(
            "NO_MANIFESTS",
            format!("No module manifests found in {}", surfer_dir.display()),
        )
        .into());
    }

    Ok(ClasspathManifest {
        gradle_version: String::new(),
        extraction_timestamp: String::new(),
        modules,
    })
}

/// Deduplicate dependencies across all modules by GAV.
pub fn deduplicate(manifest: &ClasspathManifest) -> Vec<DependencyInfo> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for dep in manifest.all_dependencies() {
        let gav = dep.gav();
        if seen.insert(gav) {
            result.push(dep.clone());
        }
    }
    result
}
