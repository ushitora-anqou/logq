use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use serde_json::Value;

/// Color configuration for JSON syntax highlighting.
pub struct HighlightColors {
    pub key: Color,
    pub string: Color,
    pub number: Color,
    pub boolean: Color,
    pub null: Color,
    pub punctuation: Color,
}

impl Default for HighlightColors {
    fn default() -> Self {
        Self {
            key: Color::Cyan,
            string: Color::Green,
            number: Color::Yellow,
            boolean: Color::Magenta,
            null: Color::DarkGray,
            punctuation: Color::White,
        }
    }
}

/// Highlight a line of JSON or plain text.
/// Returns a `Text` with syntax highlighting if valid JSON, or plain text otherwise.
pub fn highlight_line(line: &str, colors: &HighlightColors) -> Text<'static> {
    match serde_json::from_str::<Value>(line) {
        Ok(value) => {
            let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| line.to_string());
            let lines: Vec<Line<'static>> = pretty
                .lines()
                .map(|l| highlight_json_line(l, colors))
                .collect();
            Text::from(lines)
        }
        Err(_) => Text::from(Line::from(Span::styled(line.to_string(), Style::default()))),
    }
}

/// Apply syntax highlighting to a single line of pretty-printed JSON.
fn highlight_json_line(line: &str, colors: &HighlightColors) -> Line<'static> {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let indent = Span::raw(line[..indent_len].to_string());

    let mut spans: Vec<Span<'static>> = vec![indent];
    let mut rest = trimmed;
    let mut in_key = true; // Before ':' we're in key, after ':' we're in value

    while !rest.is_empty() {
        if rest.starts_with(':') {
            spans.push(Span::styled(
                ":".to_string(),
                Style::default().fg(colors.punctuation),
            ));
            rest = rest[1..].trim_start();
            in_key = false;
            continue;
        }
        if rest.starts_with(',') {
            spans.push(Span::styled(
                ",".to_string(),
                Style::default().fg(colors.punctuation),
            ));
            rest = rest[1..].trim_start();
            // After comma in an object, next token is a key
            // After comma in an array, next token is a value
            // Heuristic: if next non-whitespace is a quote, it's a key
            in_key = rest.starts_with('"');
            continue;
        }
        if rest.starts_with('{')
            || rest.starts_with('}')
            || rest.starts_with('[')
            || rest.starts_with(']')
        {
            spans.push(Span::styled(
                rest[..1].to_string(),
                Style::default().fg(colors.punctuation),
            ));
            rest = rest[1..].trim_start();
            in_key = rest.starts_with('"');
            continue;
        }

        if rest.starts_with('"') {
            let end = find_string_end(rest);
            let s = &rest[..end];
            if in_key {
                spans.push(Span::styled(s.to_string(), Style::default().fg(colors.key)));
            } else {
                spans.push(Span::styled(
                    s.to_string(),
                    Style::default().fg(colors.string),
                ));
            }
            rest = rest[end..].trim_start();
            continue;
        }

        // Number, boolean, or null
        let end = rest
            .find(|c: char| c == ',' || c == '}' || c == ']' || c == ':' || c.is_whitespace())
            .unwrap_or(rest.len());
        let token = &rest[..end];
        let color = if token == "true" || token == "false" {
            colors.boolean
        } else if token == "null" {
            colors.null
        } else {
            colors.number
        };
        spans.push(Span::styled(token.to_string(), Style::default().fg(color)));
        rest = rest[end..].trim_start();
    }

    Line::from(spans)
}

/// Find the end index of a JSON string (including closing quote).
pub fn find_string_end(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 1; // Skip opening quote
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2; // Skip escape sequence
        } else if bytes[i] == b'"' {
            return i + 1;
        } else {
            i += 1;
        }
    }
    s.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_valid_object() {
        let colors = HighlightColors::default();
        let text = highlight_line("{\"name\":\"alice\",\"age\":30}", &colors);
        assert!(
            text.lines.len() > 1,
            "Pretty-printed object should be multi-line"
        );
    }

    #[test]
    fn test_highlight_valid_array() {
        let colors = HighlightColors::default();
        let text = highlight_line("[1,2,3]", &colors);
        assert!(
            text.lines.len() > 1,
            "Pretty-printed array should be multi-line"
        );
    }

    #[test]
    fn test_highlight_nested_json() {
        let colors = HighlightColors::default();
        let text = highlight_line("{\"outer\":{\"inner\":\"value\"},\"arr\":[1,2]}", &colors);
        assert!(text.lines.len() > 3);
    }

    #[test]
    fn test_highlight_invalid_json() {
        let colors = HighlightColors::default();
        let text = highlight_line("not json at all", &colors);
        assert_eq!(text.lines.len(), 1);
    }

    #[test]
    fn test_highlight_primitive_values() {
        let colors = HighlightColors::default();
        assert_eq!(highlight_line("\"hello\"", &colors).lines.len(), 1);
        assert_eq!(highlight_line("42", &colors).lines.len(), 1);
        assert_eq!(highlight_line("true", &colors).lines.len(), 1);
        assert_eq!(highlight_line("null", &colors).lines.len(), 1);
    }

    #[test]
    fn test_highlight_empty_structures() {
        let colors = HighlightColors::default();
        assert_eq!(highlight_line("{}", &colors).lines.len(), 1);
        assert_eq!(highlight_line("[]", &colors).lines.len(), 1);
    }

    #[test]
    fn test_find_string_end() {
        assert_eq!(find_string_end("\"hello\""), 7);
        assert_eq!(find_string_end("\"he\\\"llo\""), 9);
        assert_eq!(find_string_end("\"\""), 2);
    }
}
