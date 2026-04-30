use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};

use crate::highlight::{HighlightColors, find_string_end};

pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…", &s[..end])
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
            let (chunk, chunk_width) = if remaining.chars().count() <= available {
                let w = remaining.chars().count();
                let s = std::mem::take(&mut remaining);
                (s, w)
            } else {
                let mut end = available;
                while end > 0 && !remaining.is_char_boundary(end) {
                    end -= 1;
                }
                let w = remaining[..end].chars().count();
                let s = remaining[..end].to_string();
                remaining = remaining[end..].to_string();
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

/// Apply lightweight syntax highlighting to a display line for the list view.
pub fn highlight_display_line(
    line: &str,
    colors: &HighlightColors,
    _is_selected: bool,
) -> Vec<Span<'static>> {
    // Check if it looks like JSON (starts with { or [)
    let trimmed = line.trim_start();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return vec![Span::raw(line.to_string())];
    }

    // For list view, just do basic coloring: try to highlight key-value pairs
    let mut spans = Vec::new();
    let mut rest = line;

    while !rest.is_empty() {
        if rest.starts_with('"') {
            let end = find_string_end(rest);
            let s = &rest[..end];
            // Heuristic: if followed by ':', it's a key
            let after = rest[end..].trim_start();
            let is_key = after.starts_with(':');
            let color = if is_key { colors.key } else { colors.string };
            spans.push(Span::styled(s.to_string(), Style::default().fg(color)));
            rest = &rest[end..];
        } else if rest.starts_with(':')
            || rest.starts_with(',')
            || rest.starts_with('{')
            || rest.starts_with('}')
            || rest.starts_with('[')
            || rest.starts_with(']')
        {
            spans.push(Span::styled(
                rest[..1].to_string(),
                Style::default().fg(colors.punctuation),
            ));
            rest = &rest[1..];
        } else {
            // Find next special char
            let end = rest
                .find(['"', ':', ',', '{', '}', '[', ']'])
                .unwrap_or(rest.len());
            let token = &rest[..end];
            let color = if token == "true" || token == "false" {
                colors.boolean
            } else if token == "null" {
                colors.null
            } else if token.trim().parse::<f64>().is_ok() {
                colors.number
            } else {
                Color::White
            };
            spans.push(Span::styled(token.to_string(), Style::default().fg(color)));
            rest = &rest[end..];
        }
    }

    spans
}
