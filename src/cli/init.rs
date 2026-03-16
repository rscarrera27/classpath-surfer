use std::path::Path;

use anyhow::Result;

use crate::cli::refresh;
use crate::config::Config;
use crate::error::CliError;
use crate::gradle::init_script;
use crate::model::InitOutput;

/// Initialize classpath-surfer in the given project directory.
///
/// Creates the `.classpath-surfer/` directory, writes a default `config.json`,
/// installs the Gradle init script into `.gradle/init.d/`, appends
/// `.classpath-surfer/` to `.gitignore`, and runs an initial refresh.
/// Returns a summary of actions performed.
pub fn run(project_dir: &Path) -> Result<InitOutput> {
    let mut actions = Vec::new();

    // 1. Create .classpath-surfer directory
    let surfer_dir = project_dir.join(".classpath-surfer");
    std::fs::create_dir_all(&surfer_dir).map_err(|e| {
        CliError::general(
            "INIT_FAILED",
            format!("Failed to create .classpath-surfer directory: {e}"),
        )
    })?;
    eprintln!("Created {}", surfer_dir.display());
    actions.push(format!("Created {}", surfer_dir.display()));

    // 2. Write default config
    let config = Config::default();
    config.save(project_dir)?;
    eprintln!("Wrote config.json");
    actions.push("Wrote config.json".to_string());

    // 3. Install Gradle init script
    let gradle_init_dir = project_dir.join(".gradle/init.d");
    std::fs::create_dir_all(&gradle_init_dir)?;
    let init_script_path = gradle_init_dir.join("classpath-surfer.gradle");
    std::fs::write(&init_script_path, init_script::INIT_SCRIPT).map_err(|e| {
        CliError::general(
            "INIT_FAILED",
            format!("Failed to write Gradle init script: {e}"),
        )
    })?;
    eprintln!(
        "Installed Gradle init script: {}",
        init_script_path.display()
    );
    actions.push(format!(
        "Installed Gradle init script: {}",
        init_script_path.display()
    ));

    // 4. Update .gitignore
    update_gitignore(project_dir)?;
    actions.push("Updated .gitignore".to_string());

    eprintln!("Initialization complete. Running initial refresh...");

    // 5. Run initial refresh (best-effort — Gradle may not be available yet)
    let default_configs: Vec<String> = ["compileClasspath", "runtimeClasspath"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    match refresh::run(project_dir, &default_configs, false, 300) {
        Ok(refresh_output) => {
            actions.push(format!(
                "Initial refresh: {} mode, {} dependencies, {} symbols",
                refresh_output.mode,
                refresh_output.dependencies_processed,
                refresh_output.symbols_indexed
            ));
        }
        Err(e) => {
            eprintln!("Initial refresh skipped: {e}");
            actions.push(format!("Initial refresh skipped: {e}"));
        }
    }

    Ok(InitOutput { actions })
}

fn update_gitignore(project_dir: &Path) -> Result<()> {
    let gitignore_path = project_dir.join(".gitignore");
    let entry = ".classpath-surfer/";

    if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path)?;
        if content.lines().any(|line| line.trim() == entry) {
            return Ok(()); // already present
        }
        let mut new_content = content;
        if !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str(entry);
        new_content.push('\n');
        std::fs::write(&gitignore_path, new_content)?;
    } else {
        std::fs::write(&gitignore_path, format!("{entry}\n"))?;
    }

    eprintln!("Updated .gitignore");
    Ok(())
}
