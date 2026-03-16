//! 3-column Miller-style browser: Deps → Packages → Symbols.
//!
//! Renders a tri-pane layout and handles keyboard navigation between columns,
//! lazy data loading, and an integrated source code overlay.

use std::path::Path;

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
use crate::model::{DepInfo, PkgInfo, SearchQuery, SearchResult, ShowOutput, format_lang_display};
use crate::tui::show::{self, HighlightedShowOutput};

use super::{BrowserConfig, ColumnFocus};

/// Number of rows from the bottom at which to trigger loading more symbol results.
const LOAD_MORE_THRESHOLD: usize = 5;

/// Page size for symbol search results.
const SYMBOL_PAGE_SIZE: usize = 50;

/// Run the 3-column browser event loop.
pub fn run(project_dir: &Path, config: &BrowserConfig) -> Result<()> {
    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir)?;

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

    if deps.is_empty() {
        if let Some(pattern) = config.dep_query {
            eprintln!("No dependencies matching '{pattern}'.");
        } else {
            eprintln!("No dependencies found.");
        }
        return Ok(());
    }

    let mut guard = crate::tui::TerminalGuard::enter()?;
    let app_config = Config::load(project_dir).unwrap_or_default();
    let manifest: Option<ClasspathManifest> = cli::show::load_manifest(&manifest_path).ok();

    let mut state = BrowserState {
        focus: config.initial_focus,
        deps,
        dep_state: TableState::default().with_selected(Some(0)),
        packages: Vec::new(),
        pkg_state: TableState::default(),
        symbols: Vec::new(),
        symbol_state: TableState::default(),
        symbols_total: 0,
        symbols_has_more: false,
        show_state: None,
        error_message: None,
    };

    // Load initial packages for the first dep
    load_packages(&reader, &mut state, config)?;

    // If initial focus is Pkg or Symbol, ensure packages column has selection
    if config.initial_focus != ColumnFocus::Dep && !state.packages.is_empty() {
        state.pkg_state.select(Some(0));
    }

    // Load initial symbols if we have packages
    if config.initial_focus == ColumnFocus::Symbol || config.initial_focus == ColumnFocus::Pkg {
        load_symbols(&reader, &mut state, config)?;
        if config.initial_focus == ColumnFocus::Symbol && !state.symbols.is_empty() {
            state.symbol_state.select(Some(0));
        }
    }

    loop {
        guard.terminal.draw(|frame| {
            render_browser(frame, frame.area(), &mut state, config);
        })?;

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
                    KeyCode::Char('q') => break,
                    KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
                        match state.focus {
                            ColumnFocus::Dep => break, // quit
                            ColumnFocus::Pkg => {
                                state.focus = ColumnFocus::Dep;
                            }
                            ColumnFocus::Symbol => {
                                state.focus = ColumnFocus::Pkg;
                            }
                        }
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        move_right(&reader, &mut state, config)?;
                    }
                    KeyCode::Enter => {
                        match state.focus {
                            ColumnFocus::Symbol => {
                                // Open show overlay
                                let idx = state.symbol_state.selected().unwrap_or(0);
                                if idx < state.symbols.len() {
                                    state.error_message = None;
                                    match load_show_output(
                                        &state.symbols,
                                        idx,
                                        project_dir,
                                        &app_config,
                                        manifest.as_ref(),
                                    ) {
                                        Ok(show_output) => {
                                            state.show_state =
                                                Some(ShowViewState::new(show_output));
                                        }
                                        Err(e) => {
                                            state.error_message = Some(format!("{e:#}"));
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Enter in dep/pkg columns moves right
                                move_right(&reader, &mut state, config)?;
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        navigate_down(&reader, &mut state, config)?;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        navigate_up(&reader, &mut state, config)?;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

/// Move focus to the right column (does not open show overlay — that is handled by Enter).
fn move_right(
    reader: &IndexReader,
    state: &mut BrowserState,
    config: &BrowserConfig,
) -> Result<()> {
    match state.focus {
        ColumnFocus::Dep => {
            if !state.packages.is_empty() {
                state.focus = ColumnFocus::Pkg;
                if state.pkg_state.selected().is_none() {
                    state.pkg_state.select(Some(0));
                    load_symbols(reader, state, config)?;
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
        ColumnFocus::Symbol => {
            // Right arrow in symbols column does nothing; Enter opens show overlay
        }
    }
    Ok(())
}

/// Navigate down in the focused column.
fn navigate_down(
    reader: &IndexReader,
    state: &mut BrowserState,
    config: &BrowserConfig,
) -> Result<()> {
    match state.focus {
        ColumnFocus::Dep => {
            let i = state.dep_state.selected().unwrap_or(0);
            if i < state.deps.len().saturating_sub(1) {
                state.dep_state.select(Some(i + 1));
                load_packages(reader, state, config)?;
                load_symbols(reader, state, config)?;
            }
        }
        ColumnFocus::Pkg => {
            let i = state.pkg_state.selected().unwrap_or(0);
            if i < state.packages.len().saturating_sub(1) {
                state.pkg_state.select(Some(i + 1));
                load_symbols(reader, state, config)?;
            }
        }
        ColumnFocus::Symbol => {
            let i = state.symbol_state.selected().unwrap_or(0);
            // Infinite scroll
            if state.symbols_has_more
                && i + LOAD_MORE_THRESHOLD >= state.symbols.len()
                && let Err(e) = load_more_symbols(reader, state, config)
            {
                state.error_message = Some(format!("{e:#}"));
            }
            if i < state.symbols.len().saturating_sub(1) {
                state.symbol_state.select(Some(i + 1));
            }
        }
    }
    Ok(())
}

/// Navigate up in the focused column.
fn navigate_up(
    reader: &IndexReader,
    state: &mut BrowserState,
    config: &BrowserConfig,
) -> Result<()> {
    match state.focus {
        ColumnFocus::Dep => {
            let i = state.dep_state.selected().unwrap_or(0);
            if i > 0 {
                state.dep_state.select(Some(i - 1));
                load_packages(reader, state, config)?;
                load_symbols(reader, state, config)?;
            }
        }
        ColumnFocus::Pkg => {
            let i = state.pkg_state.selected().unwrap_or(0);
            if i > 0 {
                state.pkg_state.select(Some(i - 1));
                load_symbols(reader, state, config)?;
            }
        }
        ColumnFocus::Symbol => {
            let i = state.symbol_state.selected().unwrap_or(0);
            if i > 0 {
                state.symbol_state.select(Some(i - 1));
            }
        }
    }
    Ok(())
}

// --- Data loading ---

/// Load packages for the currently selected dep.
fn load_packages(
    reader: &IndexReader,
    state: &mut BrowserState,
    config: &BrowserConfig,
) -> Result<()> {
    let dep_idx = state.dep_state.selected().unwrap_or(0);
    let selected_gav = &state.deps[dep_idx].gav;

    let (mut pkgs, _) = reader.list_packages_for_dependency(selected_gav)?;

    // Apply pkg_query filter
    if let Some(pattern) = config.pkg_query {
        pkgs.retain(|(pkg, _)| cli::matches_glob_pattern(pkg, pattern));
    }

    state.packages = pkgs
        .into_iter()
        .map(|(package, symbol_count)| PkgInfo {
            package,
            symbol_count,
        })
        .collect();

    // Reset pkg selection to first if packages changed
    if state.packages.is_empty() {
        state.pkg_state = TableState::default();
    } else if state.pkg_state.selected().is_none()
        || state.pkg_state.selected().unwrap_or(0) >= state.packages.len()
    {
        state.pkg_state.select(Some(0));
    }

    // Clear symbols since dep changed
    state.symbols.clear();
    state.symbol_state = TableState::default();
    state.symbols_total = 0;
    state.symbols_has_more = false;

    Ok(())
}

/// Load symbols for the currently selected dep+pkg.
fn load_symbols(
    reader: &IndexReader,
    state: &mut BrowserState,
    config: &BrowserConfig,
) -> Result<()> {
    let dep_idx = state.dep_state.selected().unwrap_or(0);
    let selected_gav = &state.deps[dep_idx].gav;

    let pkg = state
        .pkg_state
        .selected()
        .and_then(|i| state.packages.get(i))
        .map(|p| p.package.as_str());

    let sq = SearchQuery {
        query: config.symbol_query,
        symbol_types: config.symbol_types,
        limit: SYMBOL_PAGE_SIZE,
        offset: 0,
        dependency: Some(selected_gav),
        access_levels: config.access_levels,
        classpath: config.classpath,
        package: pkg,
    };

    let (results, total, _) = reader.search(&sq)?;
    state.symbols_has_more = results.len() < total;
    state.symbols_total = total;
    state.symbols = results;

    // Reset symbol selection
    if state.symbols.is_empty() {
        state.symbol_state = TableState::default();
    } else {
        state.symbol_state.select(Some(0));
    }

    Ok(())
}

/// Load more symbol results (infinite scroll).
fn load_more_symbols(
    reader: &IndexReader,
    state: &mut BrowserState,
    config: &BrowserConfig,
) -> Result<()> {
    if !state.symbols_has_more {
        return Ok(());
    }

    let dep_idx = state.dep_state.selected().unwrap_or(0);
    let selected_gav = &state.deps[dep_idx].gav;

    let pkg = state
        .pkg_state
        .selected()
        .and_then(|i| state.packages.get(i))
        .map(|p| p.package.as_str());

    let sq = SearchQuery {
        query: config.symbol_query,
        symbol_types: config.symbol_types,
        limit: SYMBOL_PAGE_SIZE,
        offset: state.symbols.len(),
        dependency: Some(selected_gav),
        access_levels: config.access_levels,
        classpath: config.classpath,
        package: pkg,
    };

    let (more, total, _) = reader.search(&sq)?;
    state.symbols_total = total;
    state.symbols_has_more = state.symbols.len() + more.len() < total;
    state.symbols.extend(more);

    Ok(())
}

/// Load show output for the selected symbol.
fn load_show_output(
    results: &[SearchResult],
    selected: usize,
    project_dir: &Path,
    config: &Config,
    manifest: Option<&ClasspathManifest>,
) -> Result<ShowOutput> {
    let result = &results[selected];

    let opts = cli::show::ShowOptions {
        fqn: &result.fqn,
        decompiler: config.decompiler,
        decompiler_jar: config.decompiler_jar.as_deref(),
        no_decompile: config.no_decompile,
        context: 50,
        full: true,
    };

    if let Some(m) = manifest {
        cli::show::load_show_output_focused(project_dir, m, &opts)
    } else {
        cli::show::run(project_dir, &opts)
    }
}

// --- State ---

/// Mutable runtime state for the 3-column browser.
struct BrowserState {
    focus: ColumnFocus,

    // Deps column
    deps: Vec<DepInfo>,
    dep_state: TableState,

    // Packages column
    packages: Vec<PkgInfo>,
    pkg_state: TableState,

    // Symbols column
    symbols: Vec<SearchResult>,
    symbol_state: TableState,
    symbols_total: usize,
    symbols_has_more: bool,

    // Show overlay
    show_state: Option<ShowViewState>,
    error_message: Option<String>,
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

// --- Show key handling ---

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

// --- Rendering ---

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

    // Split: 3 columns + detail + hint bar
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(5),
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
    let hint_text = if let Some(ref err) = state.error_message {
        Line::from(format!(" Error: {err} ")).style(Style::default().fg(Color::Red))
    } else {
        Line::from(" ←→ columns | ↑↓ navigate | Enter show source | q quit ")
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

    let rows: Vec<Row> = state
        .deps
        .iter()
        .enumerate()
        .map(|(i, dep)| {
            let selected_marker =
                if state.dep_state.selected() == Some(i) && state.focus != ColumnFocus::Dep {
                    ">"
                } else {
                    " "
                };
            Row::new(vec![
                Cell::from(format!("{selected_marker} {}", dep.gav)),
                Cell::from(dep.symbol_count.to_string()),
            ])
        })
        .collect();

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

    let rows: Vec<Row> = state
        .packages
        .iter()
        .enumerate()
        .map(|(i, pkg)| {
            let selected_marker =
                if state.pkg_state.selected() == Some(i) && state.focus != ColumnFocus::Pkg {
                    ">"
                } else {
                    " "
                };
            Row::new(vec![
                Cell::from(format!("{selected_marker} {}", pkg.package)),
                Cell::from(pkg.symbol_count.to_string()),
            ])
        })
        .collect();

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
}

/// Render the detail panel for the currently focused item.
fn render_detail_panel(frame: &mut Frame, area: Rect, state: &BrowserState) {
    let label_style = Style::default().fg(Color::DarkGray);

    let lines: Vec<Line> = match state.focus {
        ColumnFocus::Dep => {
            let dep_idx = state.dep_state.selected().unwrap_or(0);
            if let Some(dep) = state.deps.get(dep_idx) {
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
            let pkg_idx = state.pkg_state.selected().unwrap_or(0);
            if let Some(pkg) = state.packages.get(pkg_idx) {
                vec![
                    Line::from(Span::styled(pkg.package.clone(), Style::default().bold())),
                    Line::from(vec![
                        Span::styled("Symbols: ", label_style),
                        Span::raw(pkg.symbol_count.to_string()),
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
                    Span::styled("Signature: ", label_style),
                    Span::raw(
                        result
                            .signature
                            .kotlin
                            .clone()
                            .unwrap_or_else(|| result.signature.java.clone()),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("Dep: ", label_style),
                    Span::raw(result.gav.clone()),
                ]));
                lines
            } else {
                vec![]
            }
        }
    };

    let block = Block::default().borders(Borders::ALL);
    let detail = Paragraph::new(Text::from(lines)).block(block);
    frame.render_widget(detail, area);
}
