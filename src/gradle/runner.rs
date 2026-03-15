use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::error::CliError;

/// Run the Gradle `classpathSurferExport` task.
///
/// If `java_home` is provided, sets `JAVA_HOME` for the Gradle process.
pub fn run_export_task(
    project_dir: &Path,
    init_script_path: &Path,
    configurations: &[String],
    java_home: Option<&Path>,
) -> Result<()> {
    let gradle_cmd = find_gradle(project_dir);

    let configs_prop = configurations.join(",");

    let mut cmd = Command::new(&gradle_cmd);
    cmd.current_dir(project_dir)
        .arg("--init-script")
        .arg(init_script_path)
        .arg(format!("-PclasspathSurfer.configurations={configs_prop}"))
        .arg("classpathSurferExport")
        .arg("--quiet");

    if let Some(java_home) = java_home {
        cmd.env("JAVA_HOME", java_home);
    }

    let status = cmd
        .status()
        .with_context(|| format!("running {gradle_cmd}"))?;

    if !status.success() {
        return Err(CliError::transient(
            "GRADLE_FAILED",
            format!(
                "Gradle task failed with exit code: {}",
                status.code().unwrap_or(-1)
            ),
        )
        .into());
    }

    Ok(())
}

fn find_gradle(project_dir: &Path) -> String {
    // Prefer gradlew in the project directory
    let gradlew = if cfg!(windows) {
        project_dir.join("gradlew.bat")
    } else {
        project_dir.join("gradlew")
    };

    if gradlew.exists() {
        gradlew.to_string_lossy().into_owned()
    } else {
        "gradle".to_string()
    }
}
