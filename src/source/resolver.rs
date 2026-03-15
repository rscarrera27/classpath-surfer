use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::error::CliError;
use crate::manifest::{ClasspathManifest, DependencyInfo};
use crate::model::{ResolvedSource, SourceProvider};
use crate::parser::{classfile, jar};

/// Resolve source code for a given FQN, returning a primary view and optional secondary view.
///
/// Fallback chain:
/// 1. Extract the `.kt` or `.java` file from the dependency's source JAR.
/// 2. Decompile the `.class` file using the configured decompiler (CFR or Vineflower).
/// 3. Return an error if `no_decompile` is set and no source JAR is available.
pub fn resolve_source(
    fqn: &str,
    project_dir: &Path,
    manifest: &ClasspathManifest,
    decompiler: &str,
    decompiler_jar: Option<&Path>,
    no_decompile: bool,
) -> Result<ResolvedSource> {
    // Find which dependency contains this class
    let class_path = fqn_to_class_path(fqn);
    let dep = find_dependency_for_class(manifest, &class_path)?;
    let gav = dep.gav();

    // Try source JAR first using (package, filename) table lookup
    if let Some(source_jar) = &dep.source_jar_path
        && source_jar.exists()
    {
        // Extract SourceFile attribute from the class
        let source_file_name = jar::extract_entry(&dep.jar_path, &class_path)
            .ok()
            .and_then(|bytes| classfile::source_file_name_from_bytes(&bytes));
        let package = classfile::package_from_fqn(fqn);

        if let Some(sfn) = &source_file_name {
            let table = jar::build_source_table(source_jar)?;
            let key = (package, sfn.clone());
            if let Some(entry) = table.get(&key) {
                let content_bytes = jar::extract_entry(source_jar, &entry.path)?;
                let content = String::from_utf8_lossy(&content_bytes).into_owned();
                let language = entry.language;
                let primary = SourceProvider::SourceJar {
                    content,
                    path: entry.path.clone(),
                    language,
                };

                return Ok(ResolvedSource {
                    gav,
                    primary,
                    secondary: None,
                });
            }
        }
    }

    // Fallback to decompilation
    if no_decompile {
        return Err(CliError::resource_not_found(
            "NO_SOURCE",
            format!("No source JAR available for '{fqn}' and --no-decompile is set"),
        )
        .into());
    }

    let decompiled = decompile_cached(project_dir, &gav, fqn, decompiler, decompiler_jar, || {
        jar::extract_entry(&dep.jar_path, &class_path)
    })?;

    Ok(ResolvedSource {
        gav,
        primary: SourceProvider::Decompiler {
            content: decompiled,
        },
        secondary: None,
    })
}

/// Decompile with filesystem caching keyed by `{gav}:{fqn}:{decompiler}`.
fn decompile_cached(
    project_dir: &Path,
    gav: &str,
    fqn: &str,
    decompiler: &str,
    decompiler_jar: Option<&Path>,
    load_class_bytes: impl FnOnce() -> Result<Vec<u8>>,
) -> Result<String> {
    let cache_dir = project_dir.join(".classpath-surfer/decompile-cache");
    let cache_key = format!("{gav}:{fqn}:{decompiler}");
    let hash = hex::encode(Sha256::digest(cache_key.as_bytes()));
    let cache_file = cache_dir.join(format!("{hash}.java"));

    // Cache hit
    if let Ok(content) = std::fs::read_to_string(&cache_file) {
        return Ok(content);
    }

    // Cache miss — decompile
    let class_bytes =
        load_class_bytes().with_context(|| format!("extracting class bytes for {fqn}"))?;
    let content = super::decompiler::decompile(&class_bytes, decompiler, decompiler_jar)?;

    // Best-effort write to cache
    if std::fs::create_dir_all(&cache_dir).is_ok() {
        let _ = std::fs::write(&cache_file, &content);
    }

    Ok(content)
}

fn find_dependency_for_class<'a>(
    manifest: &'a ClasspathManifest,
    class_path: &str,
) -> Result<&'a DependencyInfo> {
    for dep in manifest.all_dependencies() {
        if !dep.jar_path.exists() {
            continue;
        }
        // Check if this JAR contains the class
        if jar::extract_entry(&dep.jar_path, class_path).is_ok() {
            return Ok(dep);
        }
    }
    Err(CliError::resource_not_found(
        "CLASS_NOT_FOUND",
        format!("Class '{class_path}' not found in any dependency JAR"),
    )
    .into())
}

/// Convert a fully qualified name to a classfile path.
///
/// `"com.google.common.collect.ImmutableList"` → `"com/google/common/collect/ImmutableList.class"`
pub fn fqn_to_class_path(fqn: &str) -> String {
    // Simple approach: replace dots with slashes, but inner class dots after class name become $
    let class_name = fqn.replace('.', "/");
    format!("{class_name}.class")
}
