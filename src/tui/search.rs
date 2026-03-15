//! TUI renderer for search results with integrated source code viewer.

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
use crate::model::{SearchQuery, SearchResult, ShowOutput, format_lang_display};
use crate::tui::show::{self, HighlightedShowOutput};

/// Minimum terminal width for side-by-side layout.
const SIDE_BY_SIDE_MIN_WIDTH: u16 = 120;

/// Number of rows from the bottom at which to trigger loading more results.
const LOAD_MORE_THRESHOLD: usize = 5;

/// Mutable search state that grows as the user scrolls.
struct SearchState {
    query: String,
    results: Vec<SearchResult>,
    total_matches: usize,
    has_more: bool,
    page_size: usize,
}

impl SearchState {
    /// Try to load the next page of results from the index.
    fn load_more(&mut self, reader: &IndexReader, query_template: &SearchQuery) -> Result<()> {
        if !self.has_more {
            return Ok(());
        }
        let mut paged_query = SearchQuery {
            offset: self.results.len(),
            ..*query_template
        };
        paged_query.limit = self.page_size;
        let (more_results, total, _) = reader.search(&paged_query)?;
        self.total_matches = total;
        self.has_more = self.results.len() + more_results.len() < total;
        self.results.extend(more_results);
        Ok(())
    }
}

/// Display search results with infinite scroll and integrated source viewer.
///
/// Opens the index directly and fetches pages of results on demand as the
/// user scrolls through the table.
pub fn run_interactive(project_dir: &Path, query: &SearchQuery) -> Result<()> {
    let mut guard = super::TerminalGuard::enter()?;
    run_interactive_with(&mut guard, project_dir, query)
}

/// Inner implementation of [`run_interactive`] that uses an externally provided terminal guard.
///
/// This allows parent TUI views (e.g. deps) to share a single `TerminalGuard`
/// while delegating to the search event loop for drill-down.
pub fn run_interactive_with(
    guard: &mut super::TerminalGuard,
    project_dir: &Path,
    query: &SearchQuery,
) -> Result<()> {
    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir)?;

    // Initial fetch
    let (initial_results, total_matches, _) = reader.search(query)?;

    if initial_results.is_empty() {
        eprintln!("No results found for '{}'.", query.query.unwrap_or("*"));
        return Ok(());
    }

    let has_more = initial_results.len() < total_matches;
    let mut state = SearchState {
        query: query.query.unwrap_or("*").to_string(),
        results: initial_results,
        total_matches,
        has_more,
        page_size: query.limit,
    };

    let mut table_state = TableState::default().with_selected(Some(0));
    let mut show_state: Option<ShowViewState> = None;
    let mut error_message: Option<String> = None;

    let config = Config::load(project_dir).unwrap_or_default();

    // Load manifest once for the TUI session (avoids repeated staleness checks)
    let manifest_path = project_dir.join(".classpath-surfer/classpath-manifest.json");
    let manifest: Option<ClasspathManifest> = cli::show::load_manifest(&manifest_path).ok();

    loop {
        let terminal_width = guard.terminal.size()?.width;
        let side_by_side = terminal_width >= SIDE_BY_SIDE_MIN_WIDTH && show_state.is_some();

        guard.terminal.draw(|frame| {
            let area = frame.area();

            if side_by_side {
                // Side-by-side: 35% search / 65% source
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
                    .split(area);

                render_search_table(
                    frame,
                    chunks[0],
                    &state,
                    &mut table_state,
                    error_message.as_deref(),
                    true,
                );

                if let Some(ref sv) = show_state {
                    show::render(
                        frame,
                        chunks[1],
                        &sv.output,
                        &sv.highlighted,
                        sv.scroll,
                        sv.showing_secondary,
                    );
                }
            } else if let Some(ref sv) = show_state {
                // Fullscreen source view
                show::render(
                    frame,
                    area,
                    &sv.output,
                    &sv.highlighted,
                    sv.scroll,
                    sv.showing_secondary,
                );
            } else {
                // Search table only
                render_search_table(
                    frame,
                    area,
                    &state,
                    &mut table_state,
                    error_message.as_deref(),
                    false,
                );
            }
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if show_state.is_some() {
                // Show view is active (fullscreen or side-by-side)
                let has_secondary = show_state
                    .as_ref()
                    .is_some_and(|sv| sv.output.secondary.is_some());
                match classify_show_key(key, has_secondary, side_by_side) {
                    ShowKeyAction::Quit => break,
                    ShowKeyAction::CloseShow => show_state = None,
                    ShowKeyAction::Scroll(delta) => {
                        if let Some(ref mut sv) = show_state {
                            if delta > 0 {
                                sv.scroll = sv.scroll.saturating_add(delta as u16);
                            } else {
                                sv.scroll = sv.scroll.saturating_sub((-delta) as u16);
                            }
                        }
                    }
                    ShowKeyAction::ToggleSecondary => {
                        if let Some(ref mut sv) = show_state {
                            sv.showing_secondary = !sv.showing_secondary;
                            sv.scroll = 0;
                        }
                    }
                    ShowKeyAction::ReloadSymbol => {
                        error_message = None;
                        match load_show_output(
                            &state.results,
                            &table_state,
                            project_dir,
                            &config,
                            manifest.as_ref(),
                        ) {
                            Ok(show_output) => {
                                show_state = Some(ShowViewState::new(show_output));
                            }
                            Err(e) => error_message = Some(format!("{e:#}")),
                        }
                    }
                    ShowKeyAction::None => {}
                }
            } else {
                // Search mode — no show view open
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Down | KeyCode::Char('j') => {
                        let i = table_state.selected().unwrap_or(0);

                        // Infinite scroll: load more BEFORE advancing so the
                        // cursor can move into the newly loaded results.
                        if state.has_more
                            && i + LOAD_MORE_THRESHOLD >= state.results.len()
                            && let Err(e) = state.load_more(&reader, query)
                        {
                            error_message = Some(format!("{e:#}"));
                        }

                        let next = if i >= state.results.len() - 1 {
                            i
                        } else {
                            i + 1
                        };
                        table_state.select(Some(next));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let i = table_state.selected().unwrap_or(0);
                        let next = if i == 0 { 0 } else { i - 1 };
                        table_state.select(Some(next));
                    }
                    KeyCode::Enter => {
                        error_message = None;
                        match load_show_output(
                            &state.results,
                            &table_state,
                            project_dir,
                            &config,
                            manifest.as_ref(),
                        ) {
                            Ok(show_output) => {
                                show_state = Some(ShowViewState::new(show_output));
                            }
                            Err(e) => error_message = Some(format!("{e:#}")),
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

/// Classified key action for the show view.
enum ShowKeyAction {
    /// Exit the entire TUI.
    Quit,
    /// Close the show view and return to search.
    CloseShow,
    /// Scroll by the given signed delta (positive = down).
    Scroll(i32),
    /// Toggle between primary and secondary source view.
    ToggleSecondary,
    /// Reload source for the currently selected search result (side-by-side only).
    ReloadSymbol,
    /// Unrecognized key — do nothing.
    None,
}

/// Map a key event in show view to a [`ShowKeyAction`].
fn classify_show_key(key: KeyEvent, has_secondary: bool, allow_reload: bool) -> ShowKeyAction {
    match key.code {
        KeyCode::Esc => ShowKeyAction::CloseShow,
        KeyCode::Char('q') => ShowKeyAction::Quit,
        KeyCode::Down | KeyCode::Char('j') => ShowKeyAction::Scroll(1),
        KeyCode::Up | KeyCode::Char('k') => ShowKeyAction::Scroll(-1),
        KeyCode::PageDown => ShowKeyAction::Scroll(20),
        KeyCode::PageUp => ShowKeyAction::Scroll(-20),
        KeyCode::Tab if has_secondary => ShowKeyAction::ToggleSecondary,
        KeyCode::Enter if allow_reload => ShowKeyAction::ReloadSymbol,
        _ => ShowKeyAction::None,
    }
}

/// State for the embedded show view, including cached syntax highlighting.
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

/// Extract the class-level FQN from a search result and load source.
///
/// Uses [`cli::show::load_show_output`] directly when a manifest is available,
/// skipping repeated staleness checks. Falls back to the full
/// [`cli::show::run`] path otherwise.
fn load_show_output(
    results: &[SearchResult],
    table_state: &TableState,
    project_dir: &Path,
    config: &Config,
    manifest: Option<&ClasspathManifest>,
) -> Result<ShowOutput> {
    let idx = table_state.selected().unwrap_or(0);
    let result = &results[idx];

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

/// Render the search results table with a detail panel for the selected row.
fn render_search_table(
    frame: &mut Frame,
    area: Rect,
    state: &SearchState,
    table_state: &mut TableState,
    error: Option<&str>,
    compact: bool,
) {
    // Adaptive detail height: 5 (compact), 6 (full), 7 (full + kotlin sigs)
    // +1 for scope line (almost always present)
    let has_kotlin = state.results.iter().any(|r| r.signature.kotlin.is_some());
    let detail_height: u16 = if compact {
        5
    } else if has_kotlin {
        7
    } else {
        6
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(detail_height),
            Constraint::Length(1),
        ])
        .split(area);
    let table_area = chunks[0];
    let detail_area = chunks[1];
    let hint_area = chunks[2];

    // --- Table ---
    let header_cells = if compact {
        vec![Cell::from("Symbol"), Cell::from("SrcLanguage")]
    } else {
        vec![
            Cell::from("Symbol"),
            Cell::from("Signature"),
            Cell::from("SrcLanguage"),
            Cell::from("Dependency"),
        ]
    };
    let header = Row::new(header_cells)
        .style(Style::default().bold())
        .bottom_margin(1);

    let rows: Vec<Row> = state
        .results
        .iter()
        .map(|r| {
            let src_cell = if r.has_source() {
                let lang_str = r.source_language.map(|l| l.to_string());
                let lang = format_lang_display(lang_str.as_deref().unwrap_or("java"));
                Cell::from(lang)
            } else {
                Cell::from(Span::styled(
                    "Decompiled",
                    Style::default().fg(Color::Yellow),
                ))
            };

            if compact {
                Row::new(vec![Cell::from(r.simple_name.clone()), src_cell])
            } else {
                Row::new(vec![
                    Cell::from(r.fqn.clone()),
                    Cell::from(
                        r.signature
                            .kotlin
                            .clone()
                            .unwrap_or_else(|| r.signature.java.clone()),
                    ),
                    src_cell,
                    Cell::from(r.gav.clone()),
                ])
            }
        })
        .collect();

    let widths = if compact {
        vec![Constraint::Percentage(70), Constraint::Percentage(30)]
    } else {
        vec![
            Constraint::Percentage(30),
            Constraint::Fill(1),
            Constraint::Length(12),
            Constraint::Percentage(20),
        ]
    };

    let loaded = state.results.len();
    let total = state.total_matches;
    let title = if state.has_more {
        format!(" Search: {} ({}/{}+ results) ", state.query, loaded, total)
    } else if total > loaded {
        format!(" Search: {} ({}/{} results) ", state.query, loaded, total)
    } else {
        format!(" Search: {} ({} results) ", state.query, loaded)
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .row_highlight_style(Style::default().reversed())
        .highlight_symbol(">> ");

    frame.render_stateful_widget(table, table_area, table_state);

    // --- Detail panel for selected row ---
    let selected_idx = table_state.selected().unwrap_or(0);
    render_detail_panel(frame, detail_area, &state.results[selected_idx], compact);

    // --- Hint bar ---
    let hint_text = if let Some(err) = error {
        Line::from(format!(" Error: {} ", err)).style(Style::default().fg(Color::Red))
    } else {
        Line::from(" ↑↓ navigate | Enter show source | q/Esc quit ").style(Style::default().dim())
    };
    frame.render_widget(hint_text, hint_area);
}

/// Render the detail panel showing full information for the selected search result.
fn render_detail_panel(frame: &mut Frame, area: Rect, result: &SearchResult, compact: bool) {
    let label_style = Style::default().fg(Color::DarkGray);

    let mut lines = vec![Line::from(Span::styled(
        result.fqn.clone(),
        Style::default().bold(),
    ))];

    if !compact {
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
    }

    // GAV
    lines.push(Line::from(vec![
        Span::styled("Dep: ", label_style),
        Span::raw(result.gav.clone()),
    ]));

    // Scope
    if !result.scopes.is_empty() {
        let scope_display = result
            .scopes
            .iter()
            .map(|s| s.strip_suffix("Classpath").unwrap_or(s))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(Line::from(vec![
            Span::styled("Scope: ", label_style),
            Span::raw(scope_display),
        ]));
    }

    // Source/Language badge in top-right corner
    let badge = if result.has_source() {
        let lang_str = result.source_language.map(|l| l.to_string());
        let lang = format_lang_display(lang_str.as_deref().unwrap_or("java"));
        Line::from(vec![Span::styled(
            format!(" {lang} "),
            Style::default().fg(Color::Green),
        )])
    } else {
        Line::from(vec![Span::styled(
            " Decompiled ",
            Style::default().fg(Color::Yellow),
        )])
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title_top(badge.right_aligned());
    let detail = Paragraph::new(Text::from(lines)).block(block);
    frame.render_widget(detail, area);
}
