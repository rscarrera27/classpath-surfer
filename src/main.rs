use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use classpath_surfer::cli;
use classpath_surfer::error::CliError;
use classpath_surfer::model::{AccessLevel, SearchQuery, SymbolKind};
use classpath_surfer::output::{self, OutputMode};
use classpath_surfer::source::decompiler::Decompiler;

/// Build a long version string including the git SHA.
fn long_version() -> &'static str {
    concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_SHA"), ")")
}

#[derive(Parser)]
#[command(name = "classpath-surfer", version, long_version = long_version())]
#[command(about = "Fast dependency symbol search for Gradle Java/Kotlin projects")]
#[command(after_help = "\
Repository: https://github.com/rscarrera27/classpath-surfer
Bug reports: https://github.com/rscarrera27/classpath-surfer/issues")]
struct Cli {
    /// Project directory
    #[arg(long, global = true, default_value = ".")]
    project_dir: PathBuf,

    /// Emit structured JSON output for AI agents and scripts
    #[arg(long, visible_alias = "json", global = true)]
    agentic: bool,

    /// Force plain text output even in a TTY
    #[arg(long, global = true)]
    plain: bool,

    /// Disable colored output
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    command: Commands,
}

/// Shared pagination arguments for commands that return lists.
#[derive(Args)]
struct Pagination {
    /// Maximum number of results
    #[arg(long, default_value_t = 50)]
    limit: usize,

    /// Number of results to skip (for pagination)
    #[arg(long, default_value_t = 0)]
    offset: usize,
}

#[derive(Subcommand)]
enum Commands {
    /// Install Gradle init script, default config, and run initial refresh
    #[command(
        long_about = "Install Gradle init script, default config, and run initial refresh.\n\n\
            Creates the .classpath-surfer/ directory, writes config.json and the Gradle\n\
            init script, updates .gitignore, and performs the first index build.",
        after_help = "\
EXAMPLES:
  classpath-surfer init
  cpsurf init --project-dir /path/to/project"
    )]
    Init,

    /// Extract classpath from Gradle and build/update the symbol index
    #[command(
        long_about = "Extract classpath from Gradle and build/update the symbol index.\n\n\
            Runs the Gradle classpathSurferExport task, merges per-module manifests,\n\
            computes a GAV-level diff, and performs incremental or full reindexing.\n\
            Skips Gradle if the index is fresh (unless --force is used).",
        after_help = "\
EXAMPLES:
  classpath-surfer refresh
  classpath-surfer refresh --force
  classpath-surfer refresh --configurations compileClasspath
  classpath-surfer refresh --timeout 600"
    )]
    Refresh {
        /// Gradle configurations to resolve (comma-separated)
        #[arg(long, value_delimiter = ',')]
        configurations: Vec<String>,

        /// Force Gradle re-run and full re-index, ignoring cached state
        #[arg(long, short = 'f')]
        force: bool,

        /// Timeout in seconds for Gradle execution (default: 300)
        #[arg(long)]
        timeout: Option<u64>,
    },

    /// Search for symbols in indexed dependencies
    #[command(
        long_about = "Search for symbols in indexed dependencies.\n\n\
            Supports fuzzy text search, exact FQN matching (--fqn), and regex patterns\n\
            (--regex). Results can be filtered by symbol type, access level, dependency\n\
            GAV pattern, and configuration scope. When --dependency is used without a\n\
            query, lists all symbols in matching dependencies.",
        after_help = "\
EXAMPLES:
  classpath-surfer search ImmutableList
  classpath-surfer search --fqn com.google.common.collect.ImmutableList
  classpath-surfer search --regex 'Immutable.*List'
  classpath-surfer search --dependency 'com.google.*:guava:*'
  classpath-surfer search ImmutableList --type class --access public,protected
  classpath-surfer search ImmutableList --agentic"
    )]
    Search {
        /// Symbol name or pattern to search (optional when --dependency is set)
        query: Option<String>,

        /// Filter by symbol type (comma-separated)
        #[arg(long, value_delimiter = ',')]
        r#type: Vec<SymbolKind>,

        /// Treat query as fully qualified name (exact match)
        #[arg(long)]
        fqn: bool,

        /// Treat query as a regex pattern
        #[arg(long)]
        regex: bool,

        /// Maximum number of results (default: 20 for agentic/plain, 50 for TUI)
        #[arg(long)]
        limit: Option<usize>,

        /// Number of results to skip (for pagination)
        #[arg(long, default_value_t = 0)]
        offset: usize,

        /// Restrict search to dependencies matching a GAV pattern (e.g., "com.google.*:guava:*")
        #[arg(long)]
        dependency: Option<String>,

        /// Filter by access level (comma-separated)
        #[arg(long, value_delimiter = ',', default_values_t = vec![AccessLevel::Public])]
        access: Vec<AccessLevel>,

        /// Filter by configuration scope (e.g., compileClasspath, runtimeClasspath)
        #[arg(long)]
        scope: Option<String>,
    },

    /// Show source code for a specific symbol
    #[command(
        long_about = "Show source code for a specific symbol.\n\n\
            Resolves the symbol from the index, loads source from a source JAR or by\n\
            decompiling the class file, and displays with line numbers. By default,\n\
            focuses on the symbol with --context lines of surrounding code. Use --full\n\
            to display the entire source file.",
        after_help = "\
EXAMPLES:
  classpath-surfer show com.google.gson.Gson
  classpath-surfer show com.google.gson.Gson.fromJson --context 10
  classpath-surfer show com.google.gson.Gson --full
  classpath-surfer show com.google.gson.Gson --no-decompile"
    )]
    Show {
        /// Fully qualified name of the symbol
        fqn: String,

        /// Decompiler to use if no source JAR available
        #[arg(long)]
        decompiler: Option<Decompiler>,

        /// Path to decompiler JAR
        #[arg(long)]
        decompiler_jar: Option<PathBuf>,

        /// Fail instead of decompiling if no source JAR
        #[arg(long)]
        no_decompile: bool,

        /// Lines of context before/after the symbol (for method FQNs)
        #[arg(long, default_value_t = 25)]
        context: usize,

        /// Show the full source file instead of focusing on the symbol
        #[arg(long)]
        full: bool,
    },

    /// List indexed dependencies with symbol counts
    #[command(
        long_about = "List indexed dependencies with symbol counts.\n\n\
            Shows GAV coordinates, symbol counts, and configuration scopes for all\n\
            indexed dependencies. Supports GAV pattern filtering and pagination.",
        after_help = "\
EXAMPLES:
  classpath-surfer deps
  classpath-surfer deps --filter 'com.google.*:*'
  classpath-surfer deps --scope compileClasspath
  classpath-surfer deps --limit 10 --offset 20"
    )]
    Deps {
        /// Filter dependencies by GAV pattern (e.g., "com.google.*:*")
        #[arg(long)]
        filter: Option<String>,

        /// Filter by configuration scope (e.g., compileClasspath, runtimeClasspath)
        #[arg(long)]
        scope: Option<String>,

        #[command(flatten)]
        pagination: Pagination,
    },

    /// Show index status
    #[command(
        long_about = "Show index status.\n\n\
            Reports initialization state, dependency counts, indexed symbol count,\n\
            staleness, and index disk size.",
        after_help = "\
EXAMPLES:
  classpath-surfer status
  classpath-surfer status --agentic"
    )]
    Status,

    /// Remove index data
    #[command(
        long_about = "Remove index data.\n\n\
            Deletes the Tantivy index directory, indexed manifest, and staleness\n\
            markers. The .classpath-surfer/ directory and config.json are preserved.\n\
            Safe to run multiple times (idempotent).",
        after_help = "\
EXAMPLES:
  classpath-surfer clean"
    )]
    Clean,
}

fn main() {
    let cli = Cli::parse();
    let output_mode = OutputMode::detect(cli.agentic, cli.plain, cli.no_color);

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

    let config = classpath_surfer::config::Config::load(&project_dir).unwrap_or_default();

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
            timeout,
        } => {
            let configs = if configurations.is_empty() {
                config.configurations.clone()
            } else {
                configurations
            };
            let timeout_secs = timeout.or(config.gradle_timeout).unwrap_or(300);
            render(
                output_mode,
                cli::refresh::run(&project_dir, &configs, force, timeout_secs),
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
            access,
            dependency,
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

            let access_levels: Vec<AccessLevel> = if access.contains(&AccessLevel::All) {
                vec![]
            } else {
                access
            };
            let is_listing = query.is_none();

            if output_mode == OutputMode::Tui {
                let effective_limit = limit.unwrap_or(50);
                let sq = SearchQuery {
                    query: query.as_deref(),
                    symbol_types: &r#type,
                    fqn_mode: fqn,
                    regex_mode: regex,
                    limit: effective_limit,
                    dependency: dependency.as_deref(),
                    access_levels: &access_levels,
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
                    symbol_types: &r#type,
                    fqn_mode: fqn,
                    regex_mode: regex,
                    limit: effective_limit,
                    dependency: dependency.as_deref(),
                    access_levels: &access_levels,
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
            let effective_decompiler = decompiler.unwrap_or(config.decompiler);
            let effective_jar = decompiler_jar.or(config.decompiler_jar.clone());
            let opts = cli::show::ShowOptions {
                fqn: &fqn,
                decompiler: effective_decompiler,
                decompiler_jar: effective_jar.as_deref(),
                no_decompile: no_decompile || config.no_decompile,
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
            pagination,
        } => {
            if output_mode == OutputMode::Tui {
                cli::require_index(&project_dir).and_then(|()| {
                    classpath_surfer::tui::deps::run(
                        &project_dir,
                        filter.as_deref(),
                        scope.as_deref(),
                    )
                })
            } else {
                render(
                    output_mode,
                    cli::deps::run(
                        &project_dir,
                        filter.as_deref(),
                        scope.as_deref(),
                        pagination.limit,
                        pagination.offset,
                    ),
                    cli::render::deps,
                    None::<fn(&_) -> anyhow::Result<()>>,
                )
            }
        }
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
