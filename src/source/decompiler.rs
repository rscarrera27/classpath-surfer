use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::error::CliError;

/// Supported decompiler backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Decompiler {
    /// CFR decompiler.
    Cfr,
    /// Vineflower (formerly FernFlower) decompiler.
    Vineflower,
}

impl Decompiler {
    /// Returns the lowercase string name (`"cfr"` or `"vineflower"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Decompiler::Cfr => "cfr",
            Decompiler::Vineflower => "vineflower",
        }
    }

    /// Returns the environment variable name for the JAR path.
    pub fn env_var(&self) -> &'static str {
        match self {
            Decompiler::Cfr => "CFR_JAR",
            Decompiler::Vineflower => "VINEFLOWER_JAR",
        }
    }
}

impl std::fmt::Display for Decompiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Decompile a `.class` file's bytes using an external decompiler.
///
/// Two decompiler backends are supported:
/// - **CFR** (`decompiler = "cfr"`) -- reads from stdin-like temp file, writes to stdout.
/// - **Vineflower** (`decompiler = "vineflower"`) -- reads a temp `.class` file and
///   writes a `.java` file into a temp output directory.
///
/// In both cases the raw `class_bytes` are written to a temporary file
/// (managed by `tempfile`) before invoking the decompiler JAR via `java -jar`.
pub fn decompile(
    class_bytes: &[u8],
    decompiler: Decompiler,
    decompiler_jar: Option<&Path>,
) -> Result<String> {
    let jar_path = decompiler_jar
        .map(|p| p.to_path_buf())
        .or_else(|| find_decompiler_jar(decompiler))
        .ok_or_else(|| {
            CliError::resource_not_found(
                "DECOMPILER_NOT_FOUND",
                format!("Decompiler JAR not found. Set --decompiler-jar or install {decompiler}."),
            )
        })?;

    // Write class bytes to a temp file
    let temp_dir = tempfile::tempdir()?;
    let class_file = temp_dir.path().join("Decompiled.class");
    std::fs::write(&class_file, class_bytes)?;

    match decompiler {
        Decompiler::Cfr => decompile_cfr(&class_file, &jar_path),
        Decompiler::Vineflower => decompile_vineflower(&class_file, &jar_path, temp_dir.path()),
    }
}

fn decompile_cfr(class_file: &Path, cfr_jar: &Path) -> Result<String> {
    let output = Command::new("java")
        .args([
            "-jar",
            &cfr_jar.to_string_lossy(),
            &class_file.to_string_lossy(),
        ])
        .output()
        .context("running CFR decompiler")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CliError::general(
            "DECOMPILATION_FAILED",
            format!("CFR decompilation failed: {stderr}"),
        )
        .into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn decompile_vineflower(
    class_file: &Path,
    vineflower_jar: &Path,
    temp_dir: &Path,
) -> Result<String> {
    let output_dir = temp_dir.join("output");
    std::fs::create_dir_all(&output_dir)?;

    let output = Command::new("java")
        .args([
            "-jar",
            &vineflower_jar.to_string_lossy(),
            &class_file.to_string_lossy(),
            &output_dir.to_string_lossy(),
        ])
        .output()
        .context("running Vineflower decompiler")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CliError::general(
            "DECOMPILATION_FAILED",
            format!("Vineflower decompilation failed: {stderr}"),
        )
        .into());
    }

    // Vineflower recreates the package directory structure in the output directory
    // (e.g. com/example/Foo.java), so we need to search recursively.
    let output_file = find_java_file_recursive(&output_dir)?;

    match output_file {
        Some(path) => std::fs::read_to_string(&path).context("reading decompiled output"),
        None => Err(
            CliError::general("DECOMPILATION_FAILED", "Vineflower did not produce output").into(),
        ),
    }
}

/// Recursively search for the first `.java` file in a directory tree.
fn find_java_file_recursive(dir: &Path) -> Result<Option<std::path::PathBuf>> {
    for entry in std::fs::read_dir(dir).context("reading Vineflower output directory")? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_java_file_recursive(&path)? {
                return Ok(Some(found));
            }
        } else if path.extension().is_some_and(|ext| ext == "java") {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn find_decompiler_jar(decompiler: Decompiler) -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var(decompiler.env_var()) {
        let p = std::path::PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    None
}
