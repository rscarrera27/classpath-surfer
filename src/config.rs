//! Per-project configuration.
//!
//! Loads and saves a JSON config file (`.classpath-surfer/config.json`) that
//! controls decompiler selection, target Gradle configurations, and Java home.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::source::decompiler::Decompiler;

/// Default Gradle configurations to resolve.
pub const DEFAULT_CONFIGURATIONS: &[&str] = &["compileClasspath", "runtimeClasspath"];

/// Per-project configuration stored in `.classpath-surfer/config.json`.
///
/// All fields can also be overridden via CLI flags or environment variables
/// (e.g. `--decompiler`, `JAVA_HOME`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Decompiler backend to use when no source JAR is available.
    /// Defaults to [`Decompiler::Cfr`]. Also settable via `--decompiler`.
    #[serde(default = "default_decompiler")]
    pub decompiler: Decompiler,
    /// Path to the decompiler JAR. When `None`, the tool looks for `CFR_JAR` or
    /// `VINEFLOWER_JAR` environment variables. Also settable via `--decompiler-jar`.
    pub decompiler_jar: Option<PathBuf>,
    /// Gradle configurations to resolve (e.g. `compileClasspath`, `runtimeClasspath`).
    /// Also settable via `--configurations`.
    #[serde(default = "default_configurations")]
    pub configurations: Vec<String>,
    /// Override for `JAVA_HOME` used when running Gradle. Also settable via `--java-home`.
    pub java_home: Option<PathBuf>,
    /// Never decompile — fail if no source JAR is available.
    /// Defaults to `false`. Also settable via `--no-decompile`.
    #[serde(default)]
    pub no_decompile: bool,
}

fn default_decompiler() -> Decompiler {
    Decompiler::Cfr
}

fn default_configurations() -> Vec<String> {
    DEFAULT_CONFIGURATIONS
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            decompiler: default_decompiler(),
            decompiler_jar: None,
            configurations: default_configurations(),
            java_home: None,
            no_decompile: false,
        }
    }
}

impl Config {
    /// Load configuration from `.classpath-surfer/config.json`.
    ///
    /// Returns `Config::default()` when the file does not exist.
    /// Returns [`Config::default()`] when the file does not exist.
    pub fn load(project_dir: &Path) -> Result<Self> {
        let config_path = project_dir.join(".classpath-surfer/config.json");
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("reading {}", config_path.display()))?;
            serde_json::from_str(&content)
                .with_context(|| format!("parsing {}", config_path.display()))
        } else {
            Ok(Self::default())
        }
    }

    /// Save configuration to `.classpath-surfer/config.json`.
    ///
    /// Creates the `.classpath-surfer/` directory if it does not already exist.
    pub fn save(&self, project_dir: &Path) -> Result<()> {
        let dir = project_dir.join(".classpath-surfer");
        std::fs::create_dir_all(&dir)?;
        let config_path = dir.join("config.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&config_path, content)
            .with_context(|| format!("writing {}", config_path.display()))
    }
}
