use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use unicode_truncate::UnicodeTruncateStr;
use unicode_width::UnicodeWidthStr;

use crate::highlight::{HighlightColors, JsonTokenKind, iter_json_tokens, json_literal_color};

/// Return the Unicode display width of a string.
pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

pub fn truncate_str(s: &str, max_len: usize) -> String {
    let width = display_width(s);
    if width <= max_len {
        s.to_string()
    } else {
        let (truncated, _) = s.unicode_truncate(max_len - 1);
        format!("{truncated}…")
    }
}

pub fn wrapped_text_height(text: &Text, width: usize) -> usize {
    if width == 0 {
        return text.lines.len().max(1);
    }
    text.lines
        .iter()
        .map(|line| {
            let line_width = line.width();
            if line_width == 0 {
                1
            } else {
                line_width.div_ceil(width)
            }
        })
        .sum()
}

pub fn wrap_line(line: &Line<'_>, width: usize) -> Vec<Vec<Span<'static>>> {
    let line_width = line.width();
    if width == 0 || line_width == 0 || line_width <= width {
        let spans: Vec<Span<'static>> = line
            .spans
            .iter()
            .map(|s| Span::styled(s.content.to_string(), s.style))
            .collect();
        return if spans.is_empty() {
            vec![]
        } else {
            vec![spans]
        };
    }

    let mut result = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut current_width = 0usize;

    for span in &line.spans {
        let mut remaining: String = span.content.clone().into();
        let style = span.style;
        while !remaining.is_empty() {
            let available = width.saturating_sub(current_width);
            if available == 0 && !current.is_empty() {
                result.push(std::mem::take(&mut current));
                current_width = 0;
                continue;
            }
            let (chunk, chunk_width) = if display_width(&remaining) <= available {
                let w = display_width(&remaining);
                let s = std::mem::take(&mut remaining);
                (s, w)
            } else {
                let (truncated, w) = remaining.unicode_truncate(available);
                let s = truncated.to_string();
                remaining = remaining[truncated.len()..].to_string();
                (s, w)
            };
            current.push(Span::styled(chunk, style));
            current_width += chunk_width;

            if current_width >= width {
                result.push(std::mem::take(&mut current));
                current_width = 0;
            }
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

pub fn apply_selected_style(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let selected_bg = Style::default()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);
    spans
        .into_iter()
        .map(|span| Span::styled(span.content, span.style.patch(selected_bg)))
        .collect()
}

pub fn apply_context_center_style(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let context_bg = Style::default()
        .bg(Color::Cyan)
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD);
    spans
        .into_iter()
        .map(|span| Span::styled(span.content, span.style.patch(context_bg)))
        .collect()
}

/// Apply lightweight syntax highlighting to a display line for the list view.
pub fn highlight_display_line(
    line: &str,
    colors: &HighlightColors,
    _is_selected: bool,
) -> Vec<Span<'static>> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return vec![Span::raw(line.to_string())];
    }

    let mut spans = Vec::new();
    let mut iter = iter_json_tokens(line);

    while let Some((kind, token)) = iter.next() {
        match kind {
            JsonTokenKind::Punctuation => {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default().fg(colors.punctuation),
                ));
            }
            JsonTokenKind::String => {
                let after = iter.rest.trim_start();
                let is_key = after.starts_with(':');
                let color = if is_key { colors.key } else { colors.string };
                spans.push(Span::styled(token.to_string(), Style::default().fg(color)));
            }
            JsonTokenKind::Literal => {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default().fg(json_literal_color(token, colors)),
                ));
            }
        }
    }

    spans
}
