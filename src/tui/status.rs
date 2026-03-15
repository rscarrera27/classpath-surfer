//! TUI renderer for index status display.

use anyhow::Result;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::model::StatusOutput;

/// Display index status in a centered panel. Press any key to exit.
pub fn run(output: &StatusOutput) -> Result<()> {
    let mut guard = super::TerminalGuard::enter()?;

    guard.terminal.draw(|frame| {
        let area = frame.area();

        let mut lines = vec![
            Line::from(format!(
                "Initialized:        {}",
                if output.initialized { "yes" } else { "no" }
            )),
            Line::from(format!(
                "Has index:          {}",
                if output.has_index { "yes" } else { "no" }
            )),
            Line::from(format!("Dependencies:       {}", output.dependency_count)),
            Line::from(format!("  with source JARs: {}", output.with_source_jars)),
            Line::from(format!(
                "  without source:   {}",
                output.without_source_jars
            )),
        ];

        if let Some(count) = output.indexed_symbols {
            lines.push(Line::from(format!("Indexed symbols:    {}", count)));
        }

        lines.push(Line::from(format!(
            "Stale:              {}",
            if output.is_stale { "yes" } else { "no" }
        )));

        if let Some(ref size) = output.index_size {
            lines.push(Line::from(format!("Index size:         {}", size)));
        }

        lines.push(Line::from(""));
        lines.push(Line::from("Press any key to exit.").style(Style::default().dim()));

        let text = Text::from(lines);

        // Center vertically
        let block_height = text.height() as u16 + 2; // +2 for borders
        let block_width = 50.min(area.width);
        let y = (area.height.saturating_sub(block_height)) / 2;
        let x = (area.width.saturating_sub(block_width)) / 2;
        let centered = Rect::new(x, y, block_width, block_height);

        let paragraph = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Index Status "),
        );

        frame.render_widget(paragraph, centered);
    })?;

    super::wait_for_quit()?;
    Ok(())
}
