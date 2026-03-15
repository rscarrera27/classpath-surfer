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

        /// Force Gradle re-run and full re-index, ignoring cached state
        #[arg(long)]
        force: bool,
    },

    /// Search for symbols in indexed dependencies
    Search {
        /// Symbol name or pattern to search (optional when --dependency is set)
        query: Option<String>,

        /// Filter by symbol type (comma-separated) [possible values: any, class, method, field]
        #[arg(long, default_value = "any")]
        r#type: String,

        /// Treat query as fully qualified name (exact match)
        #[arg(long)]
        fqn: bool,

        /// Treat query as a regex pattern
        #[arg(long)]
        regex: bool,

        /// Maximum number of results (default: 20 for agentic/plain, 200 for TUI)
        #[arg(long)]
        limit: Option<usize>,

        /// Number of results to skip (for pagination)
        #[arg(long, default_value = "0")]
        offset: usize,

        /// Restrict search to dependencies matching a GAV pattern (e.g., "com.google.*:guava:*")
        #[arg(long)]
        dependency: Option<String>,

        /// Filter by access level (comma-separated) [possible values: public, protected, private, package_private, all]
        #[arg(long, default_value = "public")]
        access: String,

        /// Filter by configuration scope (e.g., compileClasspath, runtimeClasspath)
        #[arg(long)]
        scope: Option<String>,
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

        /// Lines of context before/after the symbol (for method FQNs)
        #[arg(long, default_value = "25")]
        context: usize,

        /// Show the full source file instead of focusing on the symbol
        #[arg(long)]
        full: bool,
    },

    /// List indexed dependencies with symbol counts
    Deps {
        /// Filter dependencies by GAV pattern (e.g., "com.google.*:*")
        #[arg(long)]
        filter: Option<String>,

        /// Filter by configuration scope (e.g., compileClasspath, runtimeClasspath)
        #[arg(long)]
        scope: Option<String>,

        /// Maximum number of results
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Number of results to skip (for pagination)
        #[arg(long, default_value = "0")]
        offset: usize,
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
            force,
        } => {
            let configs: Vec<String> = configurations
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();
            render(
                output_mode,
                cli::refresh::run(&project_dir, &configs, force),
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
            offset,
            dependency,
            access,
            scope,
        } => {
            // Validate: at least one of query or dependency must be provided
            if query.is_none() && dependency.is_none() {
                let err: anyhow::Error = CliError::usage(
                    "MISSING_QUERY",
                    "Either a search query or --dependency must be provided.",
                )
                .into();
                if output_mode == OutputMode::Agentic {
                    output::emit_json_error(&err);
                } else {
                    eprintln!("Error: {err:#}");
                    std::process::exit(2);
                }
            }

            let access_levels: Option<Vec<&str>> = if access == "all" {
                None
            } else {
                Some(access.split(',').map(|s| s.trim()).collect())
            };
            let access_refs = access_levels.as_deref();
            let is_listing = query.is_none();

            if output_mode == OutputMode::Tui {
                let effective_limit = limit.unwrap_or(50);
                let sq = SearchQuery {
                    query: query.as_deref(),
                    symbol_type: &r#type,
                    fqn_mode: fqn,
                    regex_mode: regex,
                    limit: effective_limit,
                    dependency: dependency.as_deref(),
                    access_levels: access_refs,
                    offset: 0,
                    scope: scope.as_deref(),
                };
                cli::require_index(&project_dir).and_then(|()| {
                    classpath_surfer::tui::search::run_interactive(&project_dir, &sq)
                })
            } else {
                let effective_limit = limit.unwrap_or(20);
                let sq = SearchQuery {
                    query: query.as_deref(),
                    symbol_type: &r#type,
                    fqn_mode: fqn,
                    regex_mode: regex,
                    limit: effective_limit,
                    dependency: dependency.as_deref(),
                    access_levels: access_refs,
                    offset,
                    scope: scope.as_deref(),
                };
                let plain_renderer = if is_listing {
                    cli::render::search_list
                } else {
                    cli::render::search
                };
                render(
                    output_mode,
                    cli::search::run(&project_dir, &sq),
                    plain_renderer,
                    None::<fn(&_) -> anyhow::Result<()>>,
                )
            }
        }
        Commands::Show {
            fqn,
            decompiler,
            decompiler_jar,
            no_decompile,
            context,
            full,
        } => {
            let opts = cli::show::ShowOptions {
                fqn: &fqn,
                decompiler: &decompiler,
                decompiler_jar: decompiler_jar.as_deref(),
                no_decompile,
                context,
                full: full || output_mode == OutputMode::Tui,
            };
            render(
                output_mode,
                cli::show::run(&project_dir, &opts),
                cli::render::show,
                Some(|out: &_| classpath_surfer::tui::show::run(out)),
            )
        }
        Commands::Deps {
            filter,
            scope,
            limit,
            offset,
        } => render(
            output_mode,
            cli::deps::run(
                &project_dir,
                filter.as_deref(),
                scope.as_deref(),
                limit,
                offset,
            ),
            cli::render::deps,
            None::<fn(&_) -> anyhow::Result<()>>,
        ),
        Commands::Status => render(
            output_mode,
            cli::status::run(&project_dir),
            cli::render::status,
            None::<fn(&_) -> anyhow::Result<()>>,
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
