use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::error::CliError;

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
    decompiler: &str,
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
        "cfr" => decompile_cfr(&class_file, &jar_path),
        "vineflower" => decompile_vineflower(&class_file, &jar_path, temp_dir.path()),
        _ => Err(CliError::usage(
            "INVALID_ARGUMENT",
            format!("Unknown decompiler: '{decompiler}'. Use 'cfr' or 'vineflower'."),
        )
        .into()),
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

    // Vineflower names output after the class (from bytecode), not the input filename.
    // Find the first .java file in the output directory.
    let output_file = std::fs::read_dir(&output_dir)
        .context("reading Vineflower output directory")?
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|ext| ext == "java"))
        .map(|e| e.path());

    match output_file {
        Some(path) => std::fs::read_to_string(&path).context("reading decompiled output"),
        None => Err(
            CliError::general("DECOMPILATION_FAILED", "Vineflower did not produce output").into(),
        ),
    }
}

fn find_decompiler_jar(decompiler: &str) -> Option<std::path::PathBuf> {
    // Check environment variable
    let env_var = match decompiler {
        "cfr" => "CFR_JAR",
        "vineflower" => "VINEFLOWER_JAR",
        _ => return None,
    };

    if let Ok(path) = std::env::var(env_var) {
        let p = std::path::PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    None
}
