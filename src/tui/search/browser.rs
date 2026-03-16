//! 3-column Miller-style browser: Deps → Packages → Symbols.
//!
//! Renders a tri-pane layout and handles keyboard navigation between columns,
//! lazy data loading, and an integrated source code overlay.
//!
//! All data loading runs on background threads so the UI never freezes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};

use crate::cli;
use crate::config::Config;
use crate::index::reader::IndexReader;
use crate::manifest::ClasspathManifest;
use crate::model::{
    AccessLevel, DepInfo, PkgInfo, SearchQuery, SearchResult, ShowOutput, SymbolKind,
    format_lang_display,
};
use crate::tui::show::{self, HighlightedShowOutput};

use super::{BrowserConfig, ColumnFocus};

/// Number of rows from the bottom at which to trigger loading more symbol results.
const LOAD_MORE_THRESHOLD: usize = 5;

/// Page size for symbol search results.
const SYMBOL_PAGE_SIZE: usize = 50;

// ---------------------------------------------------------------------------
// Background task types
// ---------------------------------------------------------------------------

/// Cloneable snapshot of filter configuration for background tasks.
#[derive(Clone)]
struct FilterSnapshot {
    symbol_query: Option<String>,
    pkg_query: Option<String>,
    symbol_types: Vec<SymbolKind>,
    access_levels: Vec<AccessLevel>,
    classpath: Option<String>,
}

/// Result from a background column data load.
enum LoadResult {
    /// Dep changed: new packages and symbols.
    DepChanged {
        packages: Vec<PkgInfo>,
        symbols: Vec<SearchResult>,
        symbols_total: usize,
        symbols_has_more: bool,
    },
    /// Pkg changed: new symbols.
    Symbols {
        symbols: Vec<SearchResult>,
        symbols_total: usize,
        symbols_has_more: bool,
    },
    /// Infinite scroll: append symbols.
    MoreSymbols {
        more: Vec<SearchResult>,
        symbols_total: usize,
    },
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the 3-column browser event loop.
pub fn run(project_dir: &Path, config: &BrowserConfig) -> Result<()> {
    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = Arc::new(IndexReader::open(&index_dir)?);

    // Load manifest for classpath info
    let manifest_path = project_dir.join(".classpath-surfer/classpath-manifest.json");
    let classpath_map = if manifest_path.exists() {
        let content = std::fs::read_to_string(&manifest_path)?;
        let manifest: ClasspathManifest = serde_json::from_str(&content)?;
        manifest.classpaths_by_gav()
    } else {
        std::collections::HashMap::new()
    };

    // Load all GAVs
    let all_gavs = reader.list_gavs()?;

    // Apply dep_query and classpath filters
    let filtered_gavs: Vec<&(String, usize)> = {
        let by_query: Vec<&(String, usize)> = if let Some(pattern) = config.dep_query {
            all_gavs
                .iter()
                .filter(|(gav, _)| cli::matches_glob_pattern(gav, pattern))
                .collect()
        } else {
            all_gavs.iter().collect()
        };

        if let Some(classpath_filter) = config.classpath {
            by_query
                .into_iter()
                .filter(|(gav, _)| {
                    classpath_map
                        .get(gav.as_str())
                        .is_some_and(|classpaths| classpaths.contains(classpath_filter))
                })
                .collect()
        } else {
            by_query
        }
    };

    // Build DepInfo list
    let deps: Vec<DepInfo> = filtered_gavs
        .into_iter()
        .map(|(gav, count)| {
            let classpaths: Vec<String> = classpath_map
                .get(gav.as_str())
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            DepInfo {
                gav: gav.clone(),
                symbol_count: *count,
                classpaths,
            }
        })
        .collect();

    // When pkg_query is set, filter deps to only those with at least one matching package.
    // Without this, the first dep might have no matching packages, leaving the Packages
    // column empty even though other deps do have matches (the agentic mode aggregates
    // packages across all deps, but the Miller-columns TUI loads per-dep).
    let deps = if let Some(pattern) = config.pkg_query {
        deps.into_iter()
            .filter(|dep| {
                reader
                    .list_packages_for_dependency(&dep.gav)
                    .map(|(pkgs, _)| {
                        pkgs.iter()
                            .any(|(pkg, _)| cli::matches_glob_pattern(pkg, pattern))
                    })
                    .unwrap_or(false)
            })
            .collect()
    } else {
        deps
    };

    // When symbol_query is set, further filter deps to those with at least one matching
    // symbol. Same rationale: agentic mode searches the entire index, but the TUI searches
    // per-dep, so the first dep may have zero matches.
    let deps = if let Some(query) = config.symbol_query {
        deps.into_iter()
            .filter(|dep| {
                let sq = SearchQuery {
                    query: Some(query),
                    symbol_types: config.symbol_types,
                    limit: 1,
                    offset: 0,
                    dependency: Some(&dep.gav),
                    access_levels: config.access_levels,
                    classpath: config.classpath,
                    package: config.pkg_query,
                };
                reader
                    .search(&sq)
                    .map(|(_, total, _)| total > 0)
                    .unwrap_or(false)
            })
            .collect()
    } else {
        deps
    };

    if deps.is_empty() {
        if let Some(query) = config.symbol_query {
            eprintln!("No symbols matching '{query}'.");
        } else if let Some(pattern) = config.pkg_query {
            eprintln!("No packages matching '{pattern}'.");
        } else if let Some(pattern) = config.dep_query {
            eprintln!("No dependencies matching '{pattern}'.");
        } else {
            eprintln!("No dependencies found.");
        }
        return Ok(());
    }

    let mut guard = crate::tui::TerminalGuard::enter()?;
    let app_config = Config::load(project_dir).unwrap_or_default();
    let manifest: Option<ClasspathManifest> = cli::show::load_manifest(&manifest_path).ok();

    let has_all_entry = config.initial_focus != ColumnFocus::Dep;
    let has_all_pkg_entry = config.initial_focus == ColumnFocus::Symbol;

    let filters = FilterSnapshot {
        symbol_query: config.symbol_query.map(|s| s.to_string()),
        pkg_query: config.pkg_query.map(|s| s.to_string()),
        symbol_types: config.symbol_types.to_vec(),
        access_levels: config.access_levels.to_vec(),
        classpath: config.classpath.map(|s| s.to_string()),
    };

    let mut state = BrowserState {
        focus: config.initial_focus,
        has_all_entry,
        deps,
        dep_state: TableState::default().with_selected(Some(0)),
        has_all_pkg_entry,
        packages: Vec::new(),
        pkg_state: TableState::default(),
        symbols: Vec::new(),
        symbol_state: TableState::default(),
        symbols_total: 0,
        symbols_has_more: false,
        show_state: None,
        loading: None,
        loading_show: None,
        error_message: None,
    };

    // Kick off initial load in the background
    state.loading = Some(spawn_dep_load(Arc::clone(&reader), &state, &filters));

    loop {
        // Check background column-data loading completion
        if let Some(rx) = state.loading.take() {
            match rx.try_recv() {
                Ok(Ok(result)) => apply_load_result(&mut state, result, config),
                Ok(Err(e)) => {
                    state.error_message = Some(format!("{e:#}"));
                }
                Err(mpsc::TryRecvError::Empty) => {
                    state.loading = Some(rx); // still loading
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    state.error_message = Some("Data loading failed".to_string());
                }
            }
        }

        // Check background show loading completion
        if let Some(rx) = state.loading_show.take() {
            match rx.try_recv() {
                Ok(Ok(show_output)) => {
                    state.show_state = Some(ShowViewState::new(show_output));
                }
                Ok(Err(e)) => {
                    state.error_message = Some(format!("{e:#}"));
                }
                Err(mpsc::TryRecvError::Empty) => {
                    state.loading_show = Some(rx);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    state.error_message = Some("Show loading failed".to_string());
                }
            }
        }

        guard.terminal.draw(|frame| {
            render_browser(frame, frame.area(), &mut state, config);
        })?;

        // Poll with timeout when any background task is running so we stay responsive;
        // block indefinitely otherwise to avoid busy-waiting.
        let is_loading = state.loading.is_some() || state.loading_show.is_some();
        if is_loading && !event::poll(Duration::from_millis(100))? {
            continue;
        }

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if state.show_state.is_some() {
                // Show overlay is active
                match classify_show_key(key) {
                    ShowKeyAction::Quit => break,
                    ShowKeyAction::CloseShow => state.show_state = None,
                    ShowKeyAction::Scroll(delta) => {
                        if let Some(ref mut sv) = state.show_state {
                            if delta > 0 {
                                sv.scroll = sv.scroll.saturating_add(delta as u16);
                            } else {
                                sv.scroll = sv.scroll.saturating_sub((-delta) as u16);
                            }
                        }
                    }
                    ShowKeyAction::ToggleSecondary => {
                        if let Some(ref mut sv) = state.show_state {
                            sv.showing_secondary = !sv.showing_secondary;
                            sv.scroll = 0;
                        }
                    }
                    ShowKeyAction::None => {}
                }
            } else {
                // Browser navigation
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Left | KeyCode::Char('h') => match state.focus {
                        ColumnFocus::Dep => {}
                        ColumnFocus::Pkg => state.focus = ColumnFocus::Dep,
                        ColumnFocus::Symbol => state.focus = ColumnFocus::Pkg,
                    },
                    KeyCode::Right | KeyCode::Char('l') => {
                        move_right(&reader, &mut state, &filters);
                    }
                    KeyCode::Enter => match state.focus {
                        ColumnFocus::Symbol => {
                            let idx = state.symbol_state.selected().unwrap_or(0);
                            if idx < state.symbols.len() {
                                state.error_message = None;
                                state.loading_show = Some(spawn_show_load(
                                    state.symbols[idx].fqn.clone(),
                                    project_dir.to_path_buf(),
                                    app_config.clone(),
                                    manifest.clone(),
                                ));
                            }
                        }
                        _ => {
                            move_right(&reader, &mut state, &filters);
                        }
                    },
                    KeyCode::Down | KeyCode::Char('j') => {
                        navigate_down(&reader, &mut state, &filters);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        navigate_up(&reader, &mut state, &filters);
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Navigation helpers (spawn background tasks instead of blocking)
// ---------------------------------------------------------------------------

/// Move focus to the right column.
fn move_right(reader: &Arc<IndexReader>, state: &mut BrowserState, filters: &FilterSnapshot) {
    match state.focus {
        ColumnFocus::Dep => {
            if state.pkg_row_count() > 0 {
                state.focus = ColumnFocus::Pkg;
                if state.pkg_state.selected().is_none() {
                    state.pkg_state.select(Some(0));
                    state.loading = Some(spawn_pkg_load(Arc::clone(reader), state, filters));
                }
            }
        }
        ColumnFocus::Pkg => {
            if !state.symbols.is_empty() {
                state.focus = ColumnFocus::Symbol;
                if state.symbol_state.selected().is_none() {
                    state.symbol_state.select(Some(0));
                }
            }
        }
        ColumnFocus::Symbol => {}
    }
}

/// Navigate down in the focused column.
fn navigate_down(reader: &Arc<IndexReader>, state: &mut BrowserState, filters: &FilterSnapshot) {
    match state.focus {
        ColumnFocus::Dep => {
            let i = state.dep_state.selected().unwrap_or(0);
            if i < state.dep_row_count().saturating_sub(1) {
                state.dep_state.select(Some(i + 1));
                state.loading = Some(spawn_dep_load(Arc::clone(reader), state, filters));
            }
        }
        ColumnFocus::Pkg => {
            let i = state.pkg_state.selected().unwrap_or(0);
            if i < state.pkg_row_count().saturating_sub(1) {
                state.pkg_state.select(Some(i + 1));
                state.loading = Some(spawn_pkg_load(Arc::clone(reader), state, filters));
            }
        }
        ColumnFocus::Symbol => {
            let i = state.symbol_state.selected().unwrap_or(0);
            if state.symbols_has_more && i + LOAD_MORE_THRESHOLD >= state.symbols.len() {
                state.loading = Some(spawn_scroll_load(Arc::clone(reader), state, filters));
            }
            if i < state.symbols.len().saturating_sub(1) {
                state.symbol_state.select(Some(i + 1));
            }
        }
    }
}

/// Navigate up in the focused column.
fn navigate_up(reader: &Arc<IndexReader>, state: &mut BrowserState, filters: &FilterSnapshot) {
    match state.focus {
        ColumnFocus::Dep => {
            let i = state.dep_state.selected().unwrap_or(0);
            if i > 0 {
                state.dep_state.select(Some(i - 1));
                state.loading = Some(spawn_dep_load(Arc::clone(reader), state, filters));
            }
        }
        ColumnFocus::Pkg => {
            let i = state.pkg_state.selected().unwrap_or(0);
            if i > 0 {
                state.pkg_state.select(Some(i - 1));
                state.loading = Some(spawn_pkg_load(Arc::clone(reader), state, filters));
            }
        }
        ColumnFocus::Symbol => {
            let i = state.symbol_state.selected().unwrap_or(0);
            if i > 0 {
                state.symbol_state.select(Some(i - 1));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Background data loading
// ---------------------------------------------------------------------------

/// Apply a completed background load result to the browser state.
fn apply_load_result(state: &mut BrowserState, result: LoadResult, config: &BrowserConfig) {
    match result {
        LoadResult::DepChanged {
            packages,
            symbols,
            symbols_total,
            symbols_has_more,
        } => {
            state.packages = packages;
            let pkg_rows = state.pkg_row_count();
            if pkg_rows == 0 {
                state.pkg_state = TableState::default();
            } else if state.pkg_state.selected().is_none()
                || state.pkg_state.selected().unwrap_or(0) >= pkg_rows
            {
                state.pkg_state.select(Some(0));
            }
            state.symbols = symbols;
            state.symbols_total = symbols_total;
            state.symbols_has_more = symbols_has_more;
            if state.symbols.is_empty() {
                state.symbol_state = TableState::default();
            } else if config.initial_focus == ColumnFocus::Symbol {
                state.symbol_state.select(Some(0));
            } else {
                state.symbol_state = TableState::default();
            }
        }
        LoadResult::Symbols {
            symbols,
            symbols_total,
            symbols_has_more,
        } => {
            state.symbols = symbols;
            state.symbols_total = symbols_total;
            state.symbols_has_more = symbols_has_more;
            if state.symbols.is_empty() {
                state.symbol_state = TableState::default();
            } else {
                state.symbol_state.select(Some(0));
            }
        }
        LoadResult::MoreSymbols {
            more,
            symbols_total,
        } => {
            state.symbols_has_more = state.symbols.len() + more.len() < symbols_total;
            state.symbols_total = symbols_total;
            state.symbols.extend(more);
        }
    }
}

/// Spawn a background load for when the selected dep changes.
///
/// Loads packages (filtered) and initial symbols in one shot.
fn spawn_dep_load(
    reader: Arc<IndexReader>,
    state: &BrowserState,
    filters: &FilterSnapshot,
) -> mpsc::Receiver<Result<LoadResult>> {
    let is_all = state.is_all_selected();
    let has_all_pkg = state.has_all_pkg_entry;
    let dep_gav = state.selected_dep_gav().map(|s| s.to_string());
    let all_gavs: Vec<String> = state.deps.iter().map(|d| d.gav.clone()).collect();
    let filters = filters.clone();

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| -> Result<LoadResult> {
            // Load packages
            let mut pkgs = if is_all {
                let refs: Vec<&str> = all_gavs.iter().map(|s| s.as_str()).collect();
                let (p, _) = reader.list_packages_for_gavs(&refs)?;
                p
            } else {
                let gav = dep_gav.as_deref().unwrap();
                let (p, _) = reader.list_packages_for_dependency(gav)?;
                p
            };

            if let Some(ref pattern) = filters.pkg_query {
                pkgs.retain(|(pkg, _)| cli::matches_glob_pattern(pkg, pattern));
            }
            if let Some(ref query) = filters.symbol_query {
                let dependency = dep_gav.as_deref();
                pkgs.retain(|(pkg, _)| {
                    let sq = SearchQuery {
                        query: Some(query),
                        symbol_types: &filters.symbol_types,
                        limit: 1,
                        offset: 0,
                        dependency,
                        access_levels: &filters.access_levels,
                        classpath: filters.classpath.as_deref(),
                        package: Some(pkg),
                    };
                    reader
                        .search(&sq)
                        .map(|(_, total, _)| total > 0)
                        .unwrap_or(false)
                });
            }

            let packages: Vec<PkgInfo> = pkgs
                .into_iter()
                .map(|(package, symbol_count)| PkgInfo {
                    package,
                    symbol_count,
                })
                .collect();

            // Load symbols for the first selected package (or all)
            let pkg_name = if has_all_pkg {
                None
            } else {
                packages.first().map(|p| p.package.as_str())
            };

            let sq = SearchQuery {
                query: filters.symbol_query.as_deref(),
                symbol_types: &filters.symbol_types,
                limit: SYMBOL_PAGE_SIZE,
                offset: 0,
                dependency: dep_gav.as_deref(),
                access_levels: &filters.access_levels,
                classpath: filters.classpath.as_deref(),
                package: pkg_name,
            };
            let (symbols, total, _) = reader.search(&sq)?;
            let has_more = symbols.len() < total;

            Ok(LoadResult::DepChanged {
                packages,
                symbols,
                symbols_total: total,
                symbols_has_more: has_more,
            })
        })();
        let _ = tx.send(result);
    });
    rx
}

/// Spawn a background load for when the selected package changes.
fn spawn_pkg_load(
    reader: Arc<IndexReader>,
    state: &BrowserState,
    filters: &FilterSnapshot,
) -> mpsc::Receiver<Result<LoadResult>> {
    let dep_gav = state.selected_dep_gav().map(|s| s.to_string());
    let pkg_name = state.selected_pkg_name().map(|s| s.to_string());
    let filters = filters.clone();

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| -> Result<LoadResult> {
            let sq = SearchQuery {
                query: filters.symbol_query.as_deref(),
                symbol_types: &filters.symbol_types,
                limit: SYMBOL_PAGE_SIZE,
                offset: 0,
                dependency: dep_gav.as_deref(),
                access_levels: &filters.access_levels,
                classpath: filters.classpath.as_deref(),
                package: pkg_name.as_deref(),
            };
            let (symbols, total, _) = reader.search(&sq)?;
            let has_more = symbols.len() < total;
            Ok(LoadResult::Symbols {
                symbols,
                symbols_total: total,
                symbols_has_more: has_more,
            })
        })();
        let _ = tx.send(result);
    });
    rx
}

/// Spawn a background load for infinite scroll (more symbols).
fn spawn_scroll_load(
    reader: Arc<IndexReader>,
    state: &BrowserState,
    filters: &FilterSnapshot,
) -> mpsc::Receiver<Result<LoadResult>> {
    let dep_gav = state.selected_dep_gav().map(|s| s.to_string());
    let pkg_name = state.selected_pkg_name().map(|s| s.to_string());
    let offset = state.symbols.len();
    let filters = filters.clone();

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| -> Result<LoadResult> {
            let sq = SearchQuery {
                query: filters.symbol_query.as_deref(),
                symbol_types: &filters.symbol_types,
                limit: SYMBOL_PAGE_SIZE,
                offset,
                dependency: dep_gav.as_deref(),
                access_levels: &filters.access_levels,
                classpath: filters.classpath.as_deref(),
                package: pkg_name.as_deref(),
            };
            let (more, total, _) = reader.search(&sq)?;
            Ok(LoadResult::MoreSymbols {
                more,
                symbols_total: total,
            })
        })();
        let _ = tx.send(result);
    });
    rx
}

/// Spawn a background thread to load show output (decompiler can be slow).
fn spawn_show_load(
    fqn: String,
    project_dir: PathBuf,
    config: Config,
    manifest: Option<ClasspathManifest>,
) -> mpsc::Receiver<Result<ShowOutput>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let opts = cli::show::ShowOptions {
            fqn: &fqn,
            decompiler: config.decompiler,
            decompiler_jar: config.decompiler_jar.as_deref(),
            no_decompile: config.no_decompile,
            context: 50,
            full: true,
        };
        let result = if let Some(ref m) = manifest {
            cli::show::load_show_output_focused(&project_dir, m, &opts)
        } else {
            cli::show::run(&project_dir, &opts)
        };
        let _ = tx.send(result);
    });
    rx
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Mutable runtime state for the 3-column browser.
struct BrowserState {
    focus: ColumnFocus,

    // Deps column
    /// When true, a synthetic "(All)" row is shown at index 0 of the deps column.
    has_all_entry: bool,
    deps: Vec<DepInfo>,
    dep_state: TableState,

    // Packages column
    /// When true, a synthetic "(All)" row is shown at index 0 of the packages column.
    has_all_pkg_entry: bool,
    packages: Vec<PkgInfo>,
    pkg_state: TableState,

    // Symbols column
    symbols: Vec<SearchResult>,
    symbol_state: TableState,
    symbols_total: usize,
    symbols_has_more: bool,

    // Show overlay
    show_state: Option<ShowViewState>,

    // Background loading
    /// Pending column data load (packages, symbols, or scroll).
    loading: Option<mpsc::Receiver<Result<LoadResult>>>,
    /// Pending show source load (decompiler).
    loading_show: Option<mpsc::Receiver<Result<ShowOutput>>>,
    error_message: Option<String>,
}

impl BrowserState {
    /// Total number of rows in the deps column (including "(All)" if present).
    fn dep_row_count(&self) -> usize {
        self.deps.len() + usize::from(self.has_all_entry)
    }

    /// Whether the "(All)" entry is currently selected.
    fn is_all_selected(&self) -> bool {
        self.has_all_entry && self.dep_state.selected() == Some(0)
    }

    /// GAV of the currently selected dep, or `None` if "(All)" is selected.
    fn selected_dep_gav(&self) -> Option<&str> {
        let idx = self.dep_state.selected().unwrap_or(0);
        if self.has_all_entry {
            if idx == 0 {
                None
            } else {
                self.deps.get(idx - 1).map(|d| d.gav.as_str())
            }
        } else {
            self.deps.get(idx).map(|d| d.gav.as_str())
        }
    }

    /// `DepInfo` for the currently selected dep, or `None` if "(All)".
    fn selected_dep(&self) -> Option<&DepInfo> {
        let idx = self.dep_state.selected().unwrap_or(0);
        if self.has_all_entry {
            if idx == 0 {
                None
            } else {
                self.deps.get(idx - 1)
            }
        } else {
            self.deps.get(idx)
        }
    }

    /// Total number of rows in the packages column (including "(All)" if present).
    fn pkg_row_count(&self) -> usize {
        self.packages.len() + usize::from(self.has_all_pkg_entry)
    }

    /// Whether the "(All)" package entry is currently selected.
    fn is_all_pkg_selected(&self) -> bool {
        self.has_all_pkg_entry && self.pkg_state.selected() == Some(0)
    }

    /// Package name for the currently selected package, or `None` if "(All)".
    fn selected_pkg_name(&self) -> Option<&str> {
        let idx = self.pkg_state.selected()?;
        if self.has_all_pkg_entry {
            if idx == 0 {
                None
            } else {
                self.packages.get(idx - 1).map(|p| p.package.as_str())
            }
        } else {
            self.packages.get(idx).map(|p| p.package.as_str())
        }
    }
}

/// State for the show source overlay.
struct ShowViewState {
    output: ShowOutput,
    highlighted: HighlightedShowOutput,
    scroll: u16,
    showing_secondary: bool,
}

impl ShowViewState {
    fn new(output: ShowOutput) -> Self {
        let highlighted = HighlightedShowOutput::from_show_output(&output);
        let scroll = output
            .primary
            .focus
            .as_ref()
            .map(|f| (f.symbol_line as u16).saturating_sub(crate::cli::show::FOCUS_TOP_MARGIN))
            .unwrap_or(0);
        Self {
            output,
            highlighted,
            scroll,
            showing_secondary: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Show key handling
// ---------------------------------------------------------------------------

/// Classified key action for the show overlay.
enum ShowKeyAction {
    Quit,
    CloseShow,
    Scroll(i32),
    ToggleSecondary,
    None,
}

/// Map a key event in show view to a [`ShowKeyAction`].
fn classify_show_key(key: KeyEvent) -> ShowKeyAction {
    match key.code {
        KeyCode::Esc => ShowKeyAction::CloseShow,
        KeyCode::Char('q') => ShowKeyAction::Quit,
        KeyCode::Down | KeyCode::Char('j') => ShowKeyAction::Scroll(1),
        KeyCode::Up | KeyCode::Char('k') => ShowKeyAction::Scroll(-1),
        KeyCode::PageDown => ShowKeyAction::Scroll(20),
        KeyCode::PageUp => ShowKeyAction::Scroll(-20),
        KeyCode::Tab => ShowKeyAction::ToggleSecondary,
        _ => ShowKeyAction::None,
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the full browser UI.
fn render_browser(frame: &mut Frame, area: Rect, state: &mut BrowserState, config: &BrowserConfig) {
    // If show overlay is active, render it fullscreen
    if let Some(ref sv) = state.show_state {
        show::render(
            frame,
            area,
            &sv.output,
            &sv.highlighted,
            sv.scroll,
            sv.showing_secondary,
        );
        return;
    }

    // Compute detail panel height: content lines + 2 (borders) + 1 (inner padding)
    let detail_content_lines = detail_line_count(state);
    let detail_height = (detail_content_lines as u16) + 3; // borders(2) + bottom padding(1)

    // Split: 3 columns + detail + hint bar
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(detail_height),
            Constraint::Length(1),
        ])
        .split(area);

    let columns_area = main_chunks[0];
    let detail_area = main_chunks[1];
    let hint_area = main_chunks[2];

    // 3-column layout
    let col_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(30),
            Constraint::Percentage(40),
        ])
        .split(columns_area);

    render_deps_column(frame, col_chunks[0], state, config);
    render_pkgs_column(frame, col_chunks[1], state, config);
    render_symbols_column(frame, col_chunks[2], state, config);

    // Detail panel for focused column's selected item
    render_detail_panel(frame, detail_area, state);

    // Hint bar
    let is_loading = state.loading.is_some() || state.loading_show.is_some();
    let hint_text = if is_loading {
        Line::from(" Loading... ").style(Style::default().fg(Color::Yellow))
    } else if let Some(ref err) = state.error_message {
        Line::from(format!(" Error: {err} ")).style(Style::default().fg(Color::Red))
    } else {
        Line::from(" ←→ columns | ↑↓ navigate | Enter show source | Esc/q quit ")
            .style(Style::default().dim())
    };
    frame.render_widget(hint_text, hint_area);
}

/// Render the deps column.
fn render_deps_column(
    frame: &mut Frame,
    area: Rect,
    state: &mut BrowserState,
    config: &BrowserConfig,
) {
    let is_focused = state.focus == ColumnFocus::Dep;

    let title = if let Some(pattern) = config.dep_query {
        format!(" Deps '{}' ({}) ", pattern, state.deps.len())
    } else {
        format!(" Deps ({}) ", state.deps.len())
    };

    let all_row = if state.has_all_entry {
        let total_symbols: usize = state.deps.iter().map(|d| d.symbol_count).sum();
        let selected_marker =
            if state.dep_state.selected() == Some(0) && state.focus != ColumnFocus::Dep {
                ">"
            } else {
                " "
            };
        Some(Row::new(vec![
            Cell::from(format!("{selected_marker} (All)")),
            Cell::from(total_symbols.to_string()),
        ]))
    } else {
        None
    };

    let dep_rows = state.deps.iter().enumerate().map(|(i, dep)| {
        let display_idx = if state.has_all_entry { i + 1 } else { i };
        let selected_marker =
            if state.dep_state.selected() == Some(display_idx) && state.focus != ColumnFocus::Dep {
                ">"
            } else {
                " "
            };
        Row::new(vec![
            Cell::from(format!("{selected_marker} {}", dep.gav)),
            Cell::from(dep.symbol_count.to_string()),
        ])
    });

    let rows: Vec<Row> = all_row.into_iter().chain(dep_rows).collect();

    let widths = [Constraint::Fill(1), Constraint::Length(7)];
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let table = Table::new(rows, widths)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(border_style),
        )
        .row_highlight_style(if is_focused {
            Style::default().reversed()
        } else {
            Style::default()
        })
        .highlight_symbol(">> ");

    frame.render_stateful_widget(table, area, &mut state.dep_state);
    crate::tui::render_overflow_indicators(
        frame,
        area,
        state.dep_state.offset(),
        state.dep_row_count(),
    );
}

/// Render the packages column.
fn render_pkgs_column(
    frame: &mut Frame,
    area: Rect,
    state: &mut BrowserState,
    config: &BrowserConfig,
) {
    let is_focused = state.focus == ColumnFocus::Pkg;

    let title = if let Some(pattern) = config.pkg_query {
        format!(" Packages '{}' ({}) ", pattern, state.packages.len())
    } else {
        format!(" Packages ({}) ", state.packages.len())
    };

    let all_pkg_row = if state.has_all_pkg_entry {
        let total_symbols: usize = state.packages.iter().map(|p| p.symbol_count).sum();
        let selected_marker =
            if state.pkg_state.selected() == Some(0) && state.focus != ColumnFocus::Pkg {
                ">"
            } else {
                " "
            };
        Some(Row::new(vec![
            Cell::from(format!("{selected_marker} (All)")),
            Cell::from(total_symbols.to_string()),
        ]))
    } else {
        None
    };

    let pkg_rows = state.packages.iter().enumerate().map(|(i, pkg)| {
        let display_idx = if state.has_all_pkg_entry { i + 1 } else { i };
        let selected_marker =
            if state.pkg_state.selected() == Some(display_idx) && state.focus != ColumnFocus::Pkg {
                ">"
            } else {
                " "
            };
        Row::new(vec![
            Cell::from(format!("{selected_marker} {}", pkg.package)),
            Cell::from(pkg.symbol_count.to_string()),
        ])
    });

    let rows: Vec<Row> = all_pkg_row.into_iter().chain(pkg_rows).collect();

    let widths = [Constraint::Fill(1), Constraint::Length(7)];
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let table = Table::new(rows, widths)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(border_style),
        )
        .row_highlight_style(if is_focused {
            Style::default().reversed()
        } else {
            Style::default()
        })
        .highlight_symbol(">> ");

    frame.render_stateful_widget(table, area, &mut state.pkg_state);
    crate::tui::render_overflow_indicators(
        frame,
        area,
        state.pkg_state.offset(),
        state.pkg_row_count(),
    );
}

/// Render the symbols column.
fn render_symbols_column(
    frame: &mut Frame,
    area: Rect,
    state: &mut BrowserState,
    config: &BrowserConfig,
) {
    let is_focused = state.focus == ColumnFocus::Symbol;

    let loaded = state.symbols.len();
    let total = state.symbols_total;
    let title = if let Some(query) = config.symbol_query {
        if state.symbols_has_more {
            format!(" Symbols '{query}' ({loaded}/{total}+) ")
        } else {
            format!(" Symbols '{query}' ({loaded}) ")
        }
    } else if state.symbols_has_more {
        format!(" Symbols ({loaded}/{total}+) ")
    } else {
        format!(" Symbols ({loaded}) ")
    };

    let rows: Vec<Row> = state
        .symbols
        .iter()
        .map(|r| {
            let kind_badge = match r.symbol_kind {
                crate::model::SymbolKind::Class => "C",
                crate::model::SymbolKind::Method => "M",
                crate::model::SymbolKind::Field => "F",
            };
            let src = if r.has_source() {
                let lang_str = r.source_language.map(|l| l.to_string());
                format_lang_display(lang_str.as_deref().unwrap_or("java"))
            } else {
                "Decomp"
            };
            Row::new(vec![
                Cell::from(kind_badge),
                Cell::from(r.simple_name.clone()),
                Cell::from(src),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(7),
    ];
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let table = Table::new(rows, widths)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(border_style),
        )
        .row_highlight_style(if is_focused {
            Style::default().reversed()
        } else {
            Style::default()
        })
        .highlight_symbol(">> ");

    frame.render_stateful_widget(table, area, &mut state.symbol_state);
    crate::tui::render_overflow_indicators(
        frame,
        area,
        state.symbol_state.offset(),
        state.symbols.len(),
    );
}

/// Build detail lines for the currently focused item.
fn build_detail_lines(state: &BrowserState) -> Vec<Line<'static>> {
    let label_style = Style::default().fg(Color::DarkGray);

    match state.focus {
        ColumnFocus::Dep => {
            if state.is_all_selected() {
                let total_symbols: usize = state.deps.iter().map(|d| d.symbol_count).sum();
                vec![
                    Line::from(Span::styled("(All dependencies)", Style::default().bold())),
                    Line::from(vec![
                        Span::styled("Dependencies: ", label_style),
                        Span::raw(state.deps.len().to_string()),
                    ]),
                    Line::from(vec![
                        Span::styled("Symbols: ", label_style),
                        Span::raw(total_symbols.to_string()),
                    ]),
                ]
            } else if let Some(dep) = state.selected_dep() {
                let mut lines = vec![Line::from(Span::styled(
                    dep.gav.clone(),
                    Style::default().bold(),
                ))];
                lines.push(Line::from(vec![
                    Span::styled("Symbols: ", label_style),
                    Span::raw(dep.symbol_count.to_string()),
                ]));
                if !dep.classpaths.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled("Classpaths: ", label_style),
                        Span::raw(dep.classpaths.join(", ")),
                    ]));
                }
                lines
            } else {
                vec![]
            }
        }
        ColumnFocus::Pkg => {
            if state.is_all_pkg_selected() {
                let total_symbols: usize = state.packages.iter().map(|p| p.symbol_count).sum();
                vec![
                    Line::from(Span::styled("(All packages)", Style::default().bold())),
                    Line::from(vec![
                        Span::styled("Packages: ", label_style),
                        Span::raw(state.packages.len().to_string()),
                    ]),
                    Line::from(vec![
                        Span::styled("Symbols: ", label_style),
                        Span::raw(total_symbols.to_string()),
                    ]),
                ]
            } else if let Some(pkg_name) = state.selected_pkg_name() {
                let symbol_count = state
                    .packages
                    .iter()
                    .find(|p| p.package == pkg_name)
                    .map(|p| p.symbol_count)
                    .unwrap_or(0);
                vec![
                    Line::from(Span::styled(pkg_name.to_string(), Style::default().bold())),
                    Line::from(vec![
                        Span::styled("Symbols: ", label_style),
                        Span::raw(symbol_count.to_string()),
                    ]),
                ]
            } else {
                vec![]
            }
        }
        ColumnFocus::Symbol => {
            let sym_idx = state.symbol_state.selected().unwrap_or(0);
            if let Some(result) = state.symbols.get(sym_idx) {
                let mut lines = vec![Line::from(Span::styled(
                    result.fqn.clone(),
                    Style::default().bold(),
                ))];
                lines.push(Line::from(vec![
                    Span::styled("Signature(Java): ", label_style),
                    Span::raw(result.signature.java.clone()),
                ]));
                if let Some(ref kt_sig) = result.signature.kotlin {
                    lines.push(Line::from(vec![
                        Span::styled("Signature(Kotlin): ", label_style),
                        Span::raw(kt_sig.clone()),
                    ]));
                }
                lines.push(Line::from(vec![
                    Span::styled("Dep: ", label_style),
                    Span::raw(result.gav.clone()),
                ]));
                if !result.classpaths.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled("Classpath: ", label_style),
                        Span::raw(result.classpaths.join(", ")),
                    ]));
                }
                lines
            } else {
                vec![]
            }
        }
    }
}

/// Count detail content lines for layout sizing.
fn detail_line_count(state: &BrowserState) -> usize {
    match state.focus {
        ColumnFocus::Dep => {
            if state.is_all_selected() {
                3 // title + dep count + symbol count
            } else {
                state
                    .selected_dep()
                    .map(|dep| if dep.classpaths.is_empty() { 2 } else { 3 })
                    .unwrap_or(0)
            }
        }
        ColumnFocus::Pkg => {
            if state.is_all_pkg_selected() {
                3 // title + pkg count + symbol count
            } else if state.selected_pkg_name().is_some() {
                2
            } else {
                0
            }
        }
        ColumnFocus::Symbol => {
            let sym_idx = state.symbol_state.selected().unwrap_or(0);
            state
                .symbols
                .get(sym_idx)
                .map(|r| {
                    let mut n = 3; // fqn + java sig + dep
                    if r.signature.kotlin.is_some() {
                        n += 1;
                    }
                    if !r.classpaths.is_empty() {
                        n += 1;
                    }
                    n
                })
                .unwrap_or(0)
        }
    }
}

/// Render the detail panel for the currently focused item.
fn render_detail_panel(frame: &mut Frame, area: Rect, state: &BrowserState) {
    let lines = build_detail_lines(state);
    let block = Block::default().borders(Borders::ALL);
    let detail = Paragraph::new(Text::from(lines)).block(block);
    frame.render_widget(detail, area);
}
