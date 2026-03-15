//! TUI renderer for source code display with syntax highlighting.

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::model::ShowOutput;
use crate::tui::highlight::{self, HighlightedSource};

/// Pre-highlighted primary and optional secondary views.
pub struct HighlightedShowOutput {
    /// Highlighted primary source.
    pub primary: HighlightedSource,
    /// Highlighted secondary source (e.g. decompiled Java for Kotlin).
    pub secondary: Option<HighlightedSource>,
}

impl HighlightedShowOutput {
    /// Highlight both views of a [`ShowOutput`].
    pub fn from_show_output(output: &ShowOutput) -> Self {
        let primary = highlight::highlight(&output.primary.content, &output.primary.language);
        let secondary = output
            .secondary
            .as_ref()
            .map(|s| highlight::highlight(&s.content, &s.language));
        Self { primary, secondary }
    }
}

/// Display source code in a scrollable viewer (standalone, creates its own terminal guard).
pub fn run(output: &ShowOutput) -> Result<()> {
    let highlighted = HighlightedShowOutput::from_show_output(output);
    let mut guard = super::TerminalGuard::enter()?;
    let initial_scroll = output
        .primary
        .focus
        .as_ref()
        .map(|f| (f.symbol_line as u16).saturating_sub(crate::cli::show::FOCUS_TOP_MARGIN))
        .unwrap_or(0);
    let mut scroll: u16 = initial_scroll;
    let mut showing_secondary = false;

    loop {
        guard.terminal.draw(|frame| {
            let area = frame.area();
            render_inner(
                frame,
                area,
                output,
                &highlighted,
                scroll,
                showing_secondary,
                false,
            );
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Down | KeyCode::Char('j') => scroll = scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
                KeyCode::PageDown => scroll = scroll.saturating_add(20),
                KeyCode::PageUp => scroll = scroll.saturating_sub(20),
                KeyCode::Tab if output.secondary.is_some() => {
                    showing_secondary = !showing_secondary;
                    scroll = 0;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Render source code into the given area (draw-only, embedded mode).
pub fn render(
    frame: &mut Frame,
    area: Rect,
    output: &ShowOutput,
    highlighted: &HighlightedShowOutput,
    scroll: u16,
    showing_secondary: bool,
) {
    render_inner(
        frame,
        area,
        output,
        highlighted,
        scroll,
        showing_secondary,
        true,
    );
}

/// Compute the metadata panel height for the current view (borders + content lines).
fn metadata_panel_height(view: &crate::model::SourceView) -> u16 {
    // Line 1: Language + Source type
    // Line 2: GAV
    // Line 3: Path (only if present)
    // + 2 for top/bottom borders
    if view.source.source_path().is_some() {
        5
    } else {
        4
    }
}

/// Render the metadata panel above the source code.
fn render_metadata_panel(
    frame: &mut Frame,
    area: Rect,
    output: &ShowOutput,
    view: &crate::model::SourceView,
) {
    let lang_label = if view.source.has_source() {
        match view.language.as_str() {
            "kotlin" => "Kotlin",
            "scala" => "Scala",
            "groovy" => "Groovy",
            "clojure" => "Clojure",
            "unknown" => "Unknown",
            _ => "Java",
        }
    } else {
        "Java (Decompiled)"
    };

    let mut lines = Vec::new();
    if view.source.has_source() {
        lines.push(Line::from(format!(
            " Language: {lang_label}    Source: Source JAR"
        )));
    } else {
        lines.push(Line::from(vec![
            Span::raw(format!(" Language: {lang_label}    ")),
            Span::styled("Decompiled", Style::default().fg(Color::Yellow).bold()),
        ]));
    }
    lines.push(Line::from(format!(" GAV: {}", output.gav)));
    if let Some(path) = view.source.source_path() {
        lines.push(Line::from(format!(" Path: {path}")));
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title(" Metadata "));
    frame.render_widget(paragraph, area);
}

fn render_inner(
    frame: &mut Frame,
    area: Rect,
    output: &ShowOutput,
    highlighted: &HighlightedShowOutput,
    scroll: u16,
    showing_secondary: bool,
    embedded: bool,
) {
    let view = if showing_secondary {
        output.secondary.as_ref().unwrap_or(&output.primary)
    } else {
        &output.primary
    };

    let hl = if showing_secondary {
        highlighted
            .secondary
            .as_ref()
            .unwrap_or(&highlighted.primary)
    } else {
        &highlighted.primary
    };

    let source_label = if view.source.has_source() {
        "Source"
    } else {
        "Decompiled"
    };
    let title = format!(" {} — {} ({}) ", output.fqn, source_label, view.language,);

    // Layout: metadata panel (variable) + source code + hint bar (1 line)
    let meta_height = metadata_panel_height(view);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(meta_height),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

    // Metadata panel
    render_metadata_panel(frame, chunks[0], output, view);

    // Source code
    let inner_width = chunks[1].width.saturating_sub(2);
    let wrapped = highlight::wrap_lines(hl, inner_width);

    // Clamp scroll so the last wrapped line stays visible at the bottom.
    let visible_height = chunks[1].height.saturating_sub(2) as usize; // minus borders
    let max_scroll = (wrapped.len()).saturating_sub(visible_height) as u16;
    let clamped_scroll = scroll.min(max_scroll);

    let text = Text::from(wrapped);
    let paragraph = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(title))
        .scroll((clamped_scroll, 0));

    frame.render_widget(paragraph, chunks[1]);

    // Hint bar
    let tab_hint = if output.secondary.is_some() {
        "Tab switch view | "
    } else {
        ""
    };
    let quit_hint = if embedded {
        "Esc back | q quit"
    } else {
        "q/Esc quit"
    };
    let hint = Line::from(format!(" ↑↓/PgUp/PgDn scroll | {tab_hint}{quit_hint} ",))
        .style(Style::default().dim());
    frame.render_widget(hint, chunks[2]);
}
