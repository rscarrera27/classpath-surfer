//! TUI renderer for dependency listing with search drill-down.

use std::path::Path;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};

use crate::cli;
use crate::index::reader::IndexReader;
use crate::model::{DepInfo, SearchQuery};

/// Display dependencies in an interactive table with drill-down to symbol search.
///
/// Loads all dependencies matching the optional filters, then enters an event
/// loop where the user can navigate and press Enter to drill down into a
/// dependency's symbols via the search TUI.
pub fn run(project_dir: &Path, filter: Option<&str>, scope: Option<&str>) -> Result<()> {
    // Note: require_index is called by main.rs before entering TUI, so not needed here.

    let index_dir = project_dir.join(".classpath-surfer/index");
    let reader = IndexReader::open(&index_dir)?;
    let all_gavs = reader.list_gavs()?;

    // Load manifest for scope info
    let manifest_path = project_dir.join(".classpath-surfer/classpath-manifest.json");
    let scope_map = if manifest_path.exists() {
        let content = std::fs::read_to_string(&manifest_path)?;
        let manifest: crate::manifest::ClasspathManifest = serde_json::from_str(&content)?;
        manifest.scopes_by_gav()
    } else {
        std::collections::HashMap::new()
    };

    // Apply filters
    let filtered: Vec<&(String, usize)> = if let Some(pattern) = filter {
        all_gavs
            .iter()
            .filter(|(gav, _)| cli::matches_gav_pattern(gav, pattern))
            .collect()
    } else {
        all_gavs.iter().collect()
    };

    let filtered: Vec<&(String, usize)> = if let Some(scope_filter) = scope {
        filtered
            .into_iter()
            .filter(|(gav, _)| {
                scope_map
                    .get(gav.as_str())
                    .is_some_and(|scopes| scopes.contains(scope_filter))
            })
            .collect()
    } else {
        filtered
    };

    // Build DepInfo list (no pagination — load all for TUI)
    let deps: Vec<DepInfo> = filtered
        .into_iter()
        .map(|(gav, count)| {
            let scopes: Vec<String> = scope_map
                .get(gav.as_str())
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            DepInfo {
                gav: gav.clone(),
                symbol_count: *count,
                scopes,
            }
        })
        .collect();

    if deps.is_empty() {
        if let Some(pattern) = filter {
            eprintln!("No dependencies matching '{pattern}'.");
        } else {
            eprintln!("No dependencies found.");
        }
        return Ok(());
    }

    let mut guard = super::TerminalGuard::enter()?;
    let mut table_state = TableState::default().with_selected(Some(0));

    loop {
        guard.terminal.draw(|frame| {
            render_deps_table(frame, frame.area(), &deps, &mut table_state, filter);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Down | KeyCode::Char('j') => {
                    let i = table_state.selected().unwrap_or(0);
                    if i < deps.len() - 1 {
                        table_state.select(Some(i + 1));
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let i = table_state.selected().unwrap_or(0);
                    if i > 0 {
                        table_state.select(Some(i - 1));
                    }
                }
                KeyCode::Enter => {
                    let idx = table_state.selected().unwrap_or(0);
                    let selected_gav = &deps[idx].gav;

                    let query = SearchQuery {
                        query: None,
                        symbol_types: &[],
                        fqn_mode: false,
                        regex_mode: false,
                        limit: 50,
                        offset: 0,
                        dependency: Some(selected_gav),
                        access_levels: &[],
                        scope,
                    };

                    // Drill down into search TUI, sharing our guard
                    if let Err(e) =
                        super::search::run_interactive_with(&mut guard, project_dir, &query)
                    {
                        eprintln!("Search error: {e:#}");
                    }
                    // On return (Esc from search), table_state is preserved
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Render the dependency table with a detail panel for the selected row.
fn render_deps_table(
    frame: &mut Frame,
    area: Rect,
    deps: &[DepInfo],
    table_state: &mut TableState,
    filter: Option<&str>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);
    let table_area = chunks[0];
    let detail_area = chunks[1];
    let hint_area = chunks[2];

    // --- Table ---
    let header = Row::new(vec![
        Cell::from("GAV"),
        Cell::from("Symbols"),
        Cell::from("Scopes"),
    ])
    .style(Style::default().bold())
    .bottom_margin(1);

    let rows: Vec<Row> = deps
        .iter()
        .map(|dep| {
            Row::new(vec![
                Cell::from(dep.gav.clone()),
                Cell::from(dep.symbol_count.to_string()),
                Cell::from(format_scopes(&dep.scopes)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Fill(1),
        Constraint::Length(10),
        Constraint::Length(20),
    ];

    let title = if let Some(pattern) = filter {
        format!(
            " Dependencies matching '{}' ({} total) ",
            pattern,
            deps.len()
        )
    } else {
        format!(" Dependencies ({} total) ", deps.len())
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .row_highlight_style(Style::default().reversed())
        .highlight_symbol(">> ");

    frame.render_stateful_widget(table, table_area, table_state);

    // --- Detail panel ---
    let selected_idx = table_state.selected().unwrap_or(0);
    render_detail_panel(frame, detail_area, &deps[selected_idx]);

    // --- Hint bar ---
    let hint =
        Line::from(" ↑↓ navigate | Enter show symbols | q/Esc quit ").style(Style::default().dim());
    frame.render_widget(hint, hint_area);
}

/// Render the detail panel for the selected dependency.
fn render_detail_panel(frame: &mut Frame, area: Rect, dep: &DepInfo) {
    let label_style = Style::default().fg(Color::DarkGray);

    let mut lines = vec![Line::from(Span::styled(
        dep.gav.clone(),
        Style::default().bold(),
    ))];

    lines.push(Line::from(vec![
        Span::styled("Symbols: ", label_style),
        Span::raw(dep.symbol_count.to_string()),
    ]));

    if !dep.scopes.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Scopes: ", label_style),
            Span::raw(format_scopes(&dep.scopes)),
        ]));
    }

    let block = Block::default().borders(Borders::ALL);
    let detail = Paragraph::new(Text::from(lines)).block(block);
    frame.render_widget(detail, area);
}

/// Format configuration scopes for display (e.g. "compile, runtime").
fn format_scopes(scopes: &[String]) -> String {
    if scopes.is_empty() {
        return String::new();
    }
    scopes
        .iter()
        .map(|s| s.strip_suffix("Classpath").unwrap_or(s))
        .collect::<Vec<_>>()
        .join(", ")
}
