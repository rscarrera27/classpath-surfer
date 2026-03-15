use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde::Serialize;

use classpath_surfer::cli;
use classpath_surfer::error::CliError;
use classpath_surfer::model::SearchQuery;
use classpath_surfer::output::{self, OutputMode};

#[derive(Parser)]
#[command(name = "classpath-surfer", version)]
#[command(about = "Fast dependency symbol search for Gradle Java/Kotlin projects")]
struct Cli {
    /// Project directory
    #[arg(long, global = true, default_value = ".")]
    project_dir: PathBuf,

    /// Emit structured JSON output for AI agents and scripts
    #[arg(long, global = true)]
    agentic: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install Gradle init script, default config, and run initial refresh
    Init,

    /// Extract classpath from Gradle and build/update the symbol index
    Refresh {
        /// Gradle configurations to resolve (comma-separated)
        #[arg(long, default_value = "compileClasspath,runtimeClasspath")]
        configurations: String,

        /// Force full re-index (ignore previous manifest)
        #[arg(long)]
        full: bool,
    },

    /// Search for symbols in indexed dependencies
    Search {
        /// Symbol name or pattern to search
        query: String,

        /// Filter by symbol type [possible values: any, class, method, field]
        #[arg(long, default_value = "any")]
        r#type: String,

        /// Treat query as fully qualified name (exact match)
        #[arg(long)]
        fqn: bool,

        /// Treat query as a regex pattern
        #[arg(long)]
        regex: bool,

        /// Maximum number of results
        #[arg(long, default_value = "20")]
        limit: usize,

        /// Restrict search to a specific dependency (GAV pattern)
        #[arg(long)]
        dependency: Option<String>,

        /// Filter by access level (comma-separated) [possible values: public, protected, private, package_private, all]
        #[arg(long, default_value = "public")]
        access: String,
    },

    /// Show source code for a specific symbol
    Show {
        /// Fully qualified name of the symbol
        fqn: String,

        /// Decompiler to use if no source JAR available [possible values: cfr, vineflower]
        #[arg(long, default_value = "cfr")]
        decompiler: String,

        /// Path to decompiler JAR
        #[arg(long)]
        decompiler_jar: Option<PathBuf>,

        /// Fail instead of decompiling if no source JAR
        #[arg(long)]
        no_decompile: bool,
    },

    /// Show index status
    Status,

    /// Remove index data
    Clean,
}

fn main() {
    let cli = Cli::parse();
    let output_mode = OutputMode::detect(cli.agentic);

    let project_dir = match std::fs::canonicalize(&cli.project_dir) {
        Ok(p) => p,
        Err(e) => {
            let err: anyhow::Error = CliError::general(
                "INVALID_PROJECT_DIR",
                format!(
                    "Failed to resolve project directory '{}': {e}",
                    cli.project_dir.display()
                ),
            )
            .into();
            if output_mode == OutputMode::Agentic {
                output::emit_json_error(&err);
            } else {
                eprintln!("Error: {err:#}");
                std::process::exit(1);
            }
        }
    };

    let result = match cli.command {
        Commands::Init => render(
            output_mode,
            cli::init::run(&project_dir),
            cli::render::init,
            None::<fn(&_) -> anyhow::Result<()>>,
        ),
        Commands::Refresh {
            configurations,
            full,
        } => {
            let configs: Vec<String> = configurations
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();
            render(
                output_mode,
                cli::refresh::run(&project_dir, &configs, full),
                cli::render::refresh,
                None::<fn(&_) -> anyhow::Result<()>>,
            )
        }
        Commands::Search {
            query,
            r#type,
            fqn,
            regex,
            limit,
            dependency,
            access,
        } => {
            let access_levels: Option<Vec<&str>> = if access == "all" {
                None
            } else {
                Some(access.split(',').map(|s| s.trim()).collect())
            };
            let access_refs = access_levels.as_deref();
            let sq = SearchQuery {
                query: &query,
                symbol_type: &r#type,
                fqn_mode: fqn,
                regex_mode: regex,
                limit,
                dependency: dependency.as_deref(),
                access_levels: access_refs,
            };
            render(
                output_mode,
                cli::search::run(&project_dir, &sq),
                cli::render::search,
                Some(|out: &_| classpath_surfer::tui::search::run(out, &project_dir)),
            )
        }
        Commands::Show {
            fqn,
            decompiler,
            decompiler_jar,
            no_decompile,
        } => render(
            output_mode,
            cli::show::run(
                &project_dir,
                &fqn,
                &decompiler,
                decompiler_jar.as_deref(),
                no_decompile,
            ),
            cli::render::show,
            Some(|out: &_| classpath_surfer::tui::show::run(out)),
        ),
        Commands::Status => render(
            output_mode,
            cli::status::run(&project_dir),
            cli::render::status,
            Some(|out: &_| classpath_surfer::tui::status::run(out)),
        ),
        Commands::Clean => render(
            output_mode,
            cli::clean::run(&project_dir),
            cli::render::clean,
            None::<fn(&_) -> anyhow::Result<()>>,
        ),
    };

    if let Err(e) = result {
        if output_mode == OutputMode::Agentic {
            output::emit_json_error(&e);
        } else {
            eprintln!("Error: {e:#}");
            let exit_code = e
                .downcast_ref::<CliError>()
                .map(|ce| ce.exit_code)
                .unwrap_or(1);
            std::process::exit(exit_code);
        }
    }
}

/// Route command output to the appropriate renderer based on [`OutputMode`].
///
/// - `Agentic` — emit JSON to stdout.
/// - `Tui` — call the TUI renderer if provided, otherwise fall back to plain.
/// - `Plain` — call the plain-text renderer.
fn render<T: Serialize>(
    mode: OutputMode,
    result: anyhow::Result<T>,
    plain: impl FnOnce(&T),
    tui: Option<impl FnOnce(&T) -> anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let out = result?;
    match mode {
        OutputMode::Agentic => output::emit_json(&out)?,
        OutputMode::Tui => {
            if let Some(tui_fn) = tui {
                tui_fn(&out)?;
            } else {
                plain(&out);
            }
        }
        OutputMode::Plain => plain(&out),
    }
    Ok(())
}
