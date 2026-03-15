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
use crate::manifest::ClasspathManifest;
use crate::model::{SearchOutput, SearchResult, ShowOutput, SymbolKind, format_lang_display};
use crate::tui::show::{self, HighlightedShowOutput};

/// Minimum terminal width for side-by-side layout.
const SIDE_BY_SIDE_MIN_WIDTH: u16 = 120;

/// Display search results in an interactive scrollable table with source code preview.
///
/// Pressing `Enter` on a result loads and displays its source code. When the
/// terminal is wide enough (≥ 120 columns), the source is shown side-by-side;
/// otherwise it takes the full screen.
pub fn run(output: &SearchOutput, project_dir: &Path) -> Result<()> {
    if output.results.is_empty() {
        eprintln!("No results found for '{}'.", output.query);
        return Ok(());
    }

    let mut guard = super::TerminalGuard::enter()?;
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
                    output,
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
                    output,
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
                            output,
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
                        let next = if i >= output.results.len() - 1 {
                            0
                        } else {
                            i + 1
                        };
                        table_state.select(Some(next));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let i = table_state.selected().unwrap_or(0);
                        let next = if i == 0 {
                            output.results.len() - 1
                        } else {
                            i - 1
                        };
                        table_state.select(Some(next));
                    }
                    KeyCode::Enter => {
                        error_message = None;
                        match load_show_output(
                            output,
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
        Self {
            output,
            highlighted,
            scroll: 0,
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
    search_output: &SearchOutput,
    table_state: &TableState,
    project_dir: &Path,
    config: &Config,
    manifest: Option<&ClasspathManifest>,
) -> Result<ShowOutput> {
    let idx = table_state.selected().unwrap_or(0);
    let result = &search_output.results[idx];

    // For methods/fields, strip the simple_name to get the class FQN
    let class_fqn = if result.symbol_kind == SymbolKind::Class {
        result.fqn.clone()
    } else {
        result
            .fqn
            .strip_suffix(&format!(".{}", result.simple_name))
            .unwrap_or(&result.fqn)
            .to_string()
    };

    if let Some(m) = manifest {
        cli::show::load_show_output(
            project_dir,
            m,
            &class_fqn,
            &config.decompiler,
            config.decompiler_jar.as_deref(),
            config.no_decompile,
        )
    } else {
        cli::show::run(
            project_dir,
            &class_fqn,
            &config.decompiler,
            config.decompiler_jar.as_deref(),
            config.no_decompile,
        )
    }
}

/// Render the search results table with a detail panel for the selected row.
fn render_search_table(
    frame: &mut Frame,
    area: Rect,
    output: &SearchOutput,
    state: &mut TableState,
    error: Option<&str>,
    compact: bool,
) {
    // Adaptive detail height: 5 (compact), 6 (full), 7 (full + kotlin sigs)
    let has_kotlin = output.results.iter().any(|r| r.signature.kotlin.is_some());
    let detail_height: u16 = if compact {
        4
    } else if has_kotlin {
        6
    } else {
        5
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

    let rows: Vec<Row> = output
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

    let title = if output.total_matches > output.results.len() {
        format!(
            " Search: {} ({}/{} results) ",
            output.query,
            output.results.len(),
            output.total_matches
        )
    } else {
        format!(
            " Search: {} ({} results) ",
            output.query,
            output.results.len()
        )
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .row_highlight_style(Style::default().reversed())
        .highlight_symbol(">> ");

    frame.render_stateful_widget(table, table_area, state);

    // --- Detail panel for selected row ---
    let selected_idx = state.selected().unwrap_or(0);
    render_detail_panel(frame, detail_area, &output.results[selected_idx], compact);

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
