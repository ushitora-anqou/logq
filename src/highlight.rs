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
/// When the line starts with valid JSON but has trailing non-JSON content, the JSON
/// portion is highlighted and the trailing text is appended as plain lines.
pub fn highlight_line(line: &str, colors: &HighlightColors) -> Text<'static> {
    // Fast path: the entire line is valid JSON.
    match serde_json::from_str::<Value>(line) {
        Ok(value) => {
            let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| line.to_string());
            let lines: Vec<Line<'static>> = pretty
                .lines()
                .map(|l| highlight_json_line(l, colors))
                .collect();
            Text::from(lines)
        }
        Err(_) => {
            // Prefix path: try to parse one JSON value from the beginning.
            let mut stream = serde_json::Deserializer::from_str(line).into_iter::<Value>();
            match stream.next() {
                Some(Ok(value)) => {
                    let offset = stream.byte_offset();
                    let trailing = line[offset..].trim_end();
                    if offset > 0 && !trailing.is_empty() {
                        let pretty = serde_json::to_string_pretty(&value)
                            .unwrap_or_else(|_| line.to_string());
                        let mut lines: Vec<Line<'static>> = pretty
                            .lines()
                            .map(|l| highlight_json_line(l, colors))
                            .collect();
                        for tail_line in trailing.lines() {
                            lines.push(Line::from(Span::styled(
                                tail_line.to_string(),
                                Style::default(),
                            )));
                        }
                        Text::from(lines)
                    } else {
                        Text::from(Line::from(Span::styled(line.to_string(), Style::default())))
                    }
                }
                _ => Text::from(Line::from(Span::styled(line.to_string(), Style::default()))),
            }
        }
    }
}

/// Apply syntax highlighting to a single line of pretty-printed JSON.
fn highlight_json_line(line: &str, colors: &HighlightColors) -> Line<'static> {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let indent = Span::raw(line[..indent_len].to_string());

    let mut spans: Vec<Span<'static>> = vec![indent];
    let mut in_key = true;
    let mut iter = iter_json_tokens(trimmed);

    while let Some((kind, token)) = iter.next() {
        match kind {
            JsonTokenKind::Punctuation => {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default().fg(colors.punctuation),
                ));
                match token {
                    ":" => in_key = false,
                    _ => in_key = iter.rest.trim_start().starts_with('"'),
                }
            }
            JsonTokenKind::String => {
                let color = if in_key { colors.key } else { colors.string };
                spans.push(Span::styled(token.to_string(), Style::default().fg(color)));
            }
            JsonTokenKind::Literal => {
                if token.trim().is_empty() {
                    spans.push(Span::raw(token.to_string()));
                } else {
                    spans.push(Span::styled(
                        token.to_string(),
                        Style::default().fg(json_literal_color(token, colors)),
                    ));
                }
            }
        }
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JsonTokenKind {
    Punctuation,
    String,
    Literal,
}

pub struct JsonTokenIter<'a> {
    pub rest: &'a str,
}

impl<'a> Iterator for JsonTokenIter<'a> {
    type Item = (JsonTokenKind, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }

        let first = self.rest.as_bytes()[0];

        if matches!(first, b':' | b',' | b'{' | b'}' | b'[' | b']') {
            let token = &self.rest[..1];
            self.rest = &self.rest[1..];
            return Some((JsonTokenKind::Punctuation, token));
        }

        if first == b'"' {
            let end = find_string_end(self.rest);
            let token = &self.rest[..end];
            self.rest = &self.rest[end..];
            return Some((JsonTokenKind::String, token));
        }

        let end = self
            .rest
            .find(['"', ':', ',', '{', '}', '[', ']'])
            .unwrap_or(self.rest.len());
        if end == 0 {
            self.rest = &self.rest[1..];
            return self.next();
        }
        let token = &self.rest[..end];
        self.rest = &self.rest[end..];
        Some((JsonTokenKind::Literal, token))
    }
}

pub fn iter_json_tokens(input: &str) -> JsonTokenIter<'_> {
    JsonTokenIter { rest: input }
}

pub fn json_literal_color(token: &str, colors: &HighlightColors) -> Color {
    if token == "true" || token == "false" {
        colors.boolean
    } else if token == "null" {
        colors.null
    } else if token.trim().parse::<f64>().is_ok() {
        colors.number
    } else {
        Color::White
    }
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

    #[test]
    fn test_highlight_json_prefix_with_trailing_text() {
        let colors = HighlightColors::default();
        let text = highlight_line(
            r#"{"msg":"uncaught exception"}make: *** [Makefile:39: run-web2] エラー 1"#,
            &colors,
        );
        assert!(
            text.lines.len() >= 2,
            "Expected multiple lines for JSON prefix + trailing text, got {}",
            text.lines.len()
        );
        let last_line_text: String = text
            .lines
            .last()
            .unwrap()
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            last_line_text.contains("make:"),
            "Last line should contain the trailing text, got: {}",
            last_line_text
        );
    }

    #[test]
    fn test_highlight_json_array_prefix_with_tail() {
        let colors = HighlightColors::default();
        let text = highlight_line("[1,2,3]some trailing text", &colors);
        assert!(text.lines.len() >= 2);
        let last_line_text: String = text
            .lines
            .last()
            .unwrap()
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(last_line_text.contains("some trailing text"));
    }

    #[test]
    fn test_highlight_incomplete_json_is_plain() {
        let colors = HighlightColors::default();
        let text = highlight_line("{\"key\":", &colors);
        assert_eq!(text.lines.len(), 1);
    }

    #[test]
    fn test_highlight_json_prefix_with_whitespace_gap() {
        let colors = HighlightColors::default();
        let text = highlight_line("{\"a\":1}   trailing", &colors);
        assert!(text.lines.len() >= 2);
        let last_line_text: String = text
            .lines
            .last()
            .unwrap()
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(last_line_text.contains("trailing"));
    }

    #[test]
    fn test_highlight_json_prefix_with_multiline_trailing() {
        let colors = HighlightColors::default();
        let text = highlight_line("{\"a\":1}line1\nline2", &colors);
        assert!(
            text.lines.len() >= 3,
            "Expected at least 3 lines (1 JSON + 2 trailing), got {}",
            text.lines.len()
        );
    }
}
