//! Gradle classpath manifest model.
//!
//! A `ClasspathManifest` mirrors the Gradle dependency graph: it contains
//! modules (Gradle sub-projects), each with configurations (e.g. `compileClasspath`),
//! each listing resolved `DependencyInfo` entries (GAV + JAR path).
//! The `diff` and `merge` submodules support incremental re-indexing.

/// GAV-level diff between manifests for incremental re-indexing.
pub mod diff;
/// Per-module manifest merging and dependency deduplication.
pub mod merge;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level classpath manifest produced by the Gradle export task.
///
/// Mirrors the Gradle project structure: one manifest contains multiple modules
/// (Gradle subprojects), each with resolved configurations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClasspathManifest {
    /// Gradle version that produced this manifest.
    pub gradle_version: String,
    /// ISO-8601 timestamp of when the export was performed.
    pub extraction_timestamp: String,
    /// Per-module manifests (one per Gradle subproject).
    pub modules: Vec<ModuleManifest>,
}

/// Manifest for a single Gradle subproject (module).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleManifest {
    /// Gradle project path (e.g. `":app"`, `":lib:core"`).
    pub module_path: String,
    /// Resolved configurations within this module.
    pub configurations: Vec<ConfigurationManifest>,
}

/// A resolved Gradle configuration (e.g. `compileClasspath`, `runtimeClasspath`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigurationManifest {
    /// Configuration name (e.g. `"compileClasspath"`).
    pub name: String,
    /// Dependencies resolved under this configuration.
    pub dependencies: Vec<DependencyInfo>,
}

/// A single resolved dependency with its Maven coordinates and local JAR paths.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DependencyInfo {
    /// Maven group ID (e.g. `"com.google.guava"`).
    pub group: String,
    /// Maven artifact ID (e.g. `"guava"`).
    pub artifact: String,
    /// Resolved version string (e.g. `"33.0-jre"`).
    pub version: String,
    /// Absolute path to the binary (classes) JAR on disk.
    pub jar_path: PathBuf,
    /// Absolute path to the source JAR, if available.
    pub source_jar_path: Option<PathBuf>,
    /// The Gradle configuration that pulled in this dependency (e.g. `"compileClasspath"`).
    pub scope: String,
}

impl DependencyInfo {
    /// Returns the Maven GAV string (`group:artifact:version`).
    pub fn gav(&self) -> String {
        format!("{}:{}:{}", self.group, self.artifact, self.version)
    }
}

impl ClasspathManifest {
    /// Collect all unique dependencies across all modules/configurations.
    pub fn all_dependencies(&self) -> Vec<&DependencyInfo> {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for module in &self.modules {
            for config in &module.configurations {
                for dep in &config.dependencies {
                    let gav = dep.gav();
                    if seen.insert(gav) {
                        result.push(dep);
                    }
                }
            }
        }
        result
    }
}
