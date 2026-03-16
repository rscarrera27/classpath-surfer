//! Ratatui-based interactive TUI renderers.
//!
//! Provides alternate-screen terminal setup/teardown with a RAII guard,
//! and per-command TUI views for search results, source code, and status.

use std::io::{self, Stdout};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;

pub mod highlight;
pub mod search;
pub mod show;

/// A terminal wrapper that restores the original state on drop.
pub struct TerminalGuard {
    /// The ratatui terminal handle.
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    /// Enter raw mode and switch to the alternate screen.
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

/// Wait for a quit key (`q` or `Esc`) press event.
pub fn wait_for_quit() -> Result<()> {
    loop {
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            return Ok(());
        }
    }
}
