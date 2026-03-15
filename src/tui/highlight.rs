//! Syntax highlighting for source code using syntect.

use std::sync::LazyLock;

use ratatui::prelude::*;
use syntect::easy::HighlightLines;
use syntect::highlighting::{self, ThemeSet};
use syntect::parsing::{SyntaxDefinition, SyntaxSet, SyntaxSetBuilder};
use syntect::util::LinesWithEndings;

/// Width of the line number gutter (`"   1 │ "`).
const LINE_NUM_WIDTH: usize = 7;

/// Vendored Kotlin syntax definition (from bat project).
const KOTLIN_SYNTAX_YAML: &str = include_str!("../../vendor/Kotlin.sublime-syntax");

/// Default syntax set (Java, etc.).
static DEFAULT_SS: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

/// Syntax set containing Kotlin.
static KOTLIN_SS: LazyLock<SyntaxSet> = LazyLock::new(|| {
    let mut builder = SyntaxSetBuilder::new();
    builder.add_plain_text_syntax();
    if let Ok(kotlin) = SyntaxDefinition::load_from_str(KOTLIN_SYNTAX_YAML, true, None) {
        builder.add(kotlin);
    }
    builder.build()
});

/// Default theme.
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Pre-highlighted source code lines, ready for rendering.
pub struct HighlightedSource {
    /// Each element is a source line represented as a sequence of styled spans.
    /// The first span is always the line number gutter.
    pub lines: Vec<Line<'static>>,
}

/// Highlight source code and return ratatui-ready styled lines with line numbers.
pub fn highlight(source: &str, language: &str) -> HighlightedSource {
    // Expand tabs to 4 spaces so every character occupies a predictable number
    // of terminal columns. Without this, ratatui's cell-based buffer treats each
    // tab as 1 column while the terminal renders it as up to 8, causing the
    // wrap_lines width calculation to undercount and lines to overflow the panel.
    let source = source.replace('\t', "    ");

    let theme = &THEME_SET.themes["base16-eighties.dark"];

    let (ss, extension) = match language {
        "kotlin" => (&*KOTLIN_SS, "kt"),
        _ => (&*DEFAULT_SS, "java"),
    };

    let syntax = ss
        .find_syntax_by_extension(extension)
        .unwrap_or_else(|| ss.find_syntax_plain_text());

    let mut h = HighlightLines::new(syntax, theme);
    let mut lines = Vec::new();

    for (i, line) in LinesWithEndings::from(&source).enumerate() {
        let line_num_span = Span::styled(
            format!("{:>4} │ ", i + 1),
            Style::default().fg(Color::DarkGray),
        );

        let mut spans = vec![line_num_span];

        // syntect highlighting; fall back to plain on error
        if let Ok(ranges) = h.highlight_line(line, ss) {
            for (style, text) in ranges {
                let text = text.trim_end_matches('\n').to_string();
                if !text.is_empty() {
                    spans.push(Span::styled(text, syntect_to_ratatui(style)));
                }
            }
        } else {
            spans.push(Span::raw(line.trim_end_matches('\n').to_string()));
        }

        lines.push(Line::from(spans));
    }

    HighlightedSource { lines }
}

/// Wrap highlighted source lines to fit within `total_width`.
///
/// Each source line that exceeds the available code width is split into multiple
/// visual lines. The first visual line keeps its line number; continuation lines
/// receive a blank gutter (`"     │ "`) to keep the separator aligned.
pub fn wrap_lines(source: &HighlightedSource, total_width: u16) -> Vec<Line<'static>> {
    let total = total_width as usize;
    // Available width for code content (after line number gutter)
    let code_width = total.saturating_sub(LINE_NUM_WIDTH);
    if code_width == 0 {
        return source.lines.clone();
    }

    let continuation = Span::styled(
        " ".repeat(LINE_NUM_WIDTH),
        Style::default().fg(Color::DarkGray),
    );

    let mut result = Vec::new();

    for line in &source.lines {
        // First span is the line number gutter; the rest are code spans
        let gutter = &line.spans[0];
        let code_spans = &line.spans[1..];

        // Fast path: calculate total code char width
        let total_code_len: usize = code_spans.iter().map(|s| s.content.len()).sum();
        if total_code_len <= code_width {
            result.push(line.clone());
            continue;
        }

        // Slow path: split code spans across multiple visual lines
        let mut current_spans: Vec<Span<'static>> = vec![gutter.clone()];
        let mut current_width: usize = 0;

        for span in code_spans {
            let style = span.style;
            let mut remaining: &str = &span.content;

            while !remaining.is_empty() {
                let available = code_width.saturating_sub(current_width);
                if available == 0 {
                    // Flush current line, start continuation
                    result.push(Line::from(current_spans));
                    current_spans = vec![continuation.clone()];
                    current_width = 0;
                    continue;
                }

                if remaining.len() <= available {
                    current_spans.push(Span::styled(remaining.to_string(), style));
                    current_width += remaining.len();
                    break;
                }

                // Split at the available boundary (char-safe)
                let split_at = char_boundary(remaining, available);
                let (chunk, rest) = remaining.split_at(split_at);
                if !chunk.is_empty() {
                    current_spans.push(Span::styled(chunk.to_string(), style));
                }
                result.push(Line::from(current_spans));
                current_spans = vec![continuation.clone()];
                current_width = 0;
                remaining = rest;
            }
        }

        if current_spans.len() > 1 || (current_spans.len() == 1 && current_width > 0) {
            result.push(Line::from(current_spans));
        }
    }

    result
}

/// Find the largest byte offset ≤ `max_chars` that lies on a char boundary.
fn char_boundary(s: &str, max_chars: usize) -> usize {
    s.char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(s.len())
}

/// Convert a syntect foreground style to a ratatui `Style`.
fn syntect_to_ratatui(style: highlighting::Style) -> Style {
    let fg = style.foreground;
    Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b))
}
