use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::highlight::{HighlightColors, highlight_line};

const DOUBLE_CTRL_C_INTERVAL_MS: u64 = 500;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    List,
    Detail,
}

#[derive(serde::Serialize)]
pub struct AppState {
    pub lines: Vec<String>,
    pub view_mode: ViewMode,
    pub selected: usize,
    pub scroll_offset: usize,
    pub filter: Option<String>,
    pub auto_scroll: bool,
    pub detail_scroll: u16,
    pub filter_input: Option<String>,
    pub should_quit: bool,
    pub total_lines: usize,
    pub filtered_count: usize,
}

pub struct App {
    pub lines: Vec<String>,
    pub view_mode: ViewMode,
    pub selected: usize,
    pub scroll_offset: usize,
    pub filter: Option<String>,
    pub auto_scroll: bool,
    pub last_ctrl_c: Option<Instant>,
    pub detail_scroll: u16,
    pub filter_input: Option<String>,
    pub max_lines: usize,
    pub should_quit: bool,
    colors: HighlightColors,
}

impl App {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: Vec::new(),
            view_mode: ViewMode::List,
            selected: 0,
            scroll_offset: 0,
            filter: None,
            auto_scroll: true,
            last_ctrl_c: None,
            detail_scroll: 0,
            filter_input: None,
            max_lines,
            should_quit: false,
            colors: HighlightColors::default(),
        }
    }

    pub fn add_line(&mut self, line: String) {
        self.lines.push(line);
        if self.lines.len() > self.max_lines {
            self.lines.remove(0);
            if self.selected > 0 {
                self.selected -= 1;
            }
        }
        if self.auto_scroll {
            let filtered = self.filtered_indices();
            if !filtered.is_empty() {
                self.selected = filtered.len() - 1;
            }
        }
    }

    pub fn snapshot(&self) -> AppState {
        AppState {
            lines: self.lines.clone(),
            view_mode: self.view_mode.clone(),
            selected: self.selected,
            scroll_offset: self.scroll_offset,
            filter: self.filter.clone(),
            auto_scroll: self.auto_scroll,
            detail_scroll: self.detail_scroll,
            filter_input: self.filter_input.clone(),
            should_quit: self.should_quit,
            total_lines: self.lines.len(),
            filtered_count: self.filtered_indices().len(),
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        match &self.filter {
            Some(f) if !f.is_empty() => self
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.contains(f))
                .map(|(i, _)| i)
                .collect(),
            _ => (0..self.lines.len()).collect(),
        }
    }

    fn visible_height(area: &Rect) -> usize {
        // Status line is already split out in render(), so area is the list/detail area only
        area.height as usize
    }

    fn ensure_selection_visible(&mut self, visible_height: usize) {
        if self.scroll_offset > self.selected {
            self.scroll_offset = self.selected;
        }
        if self.selected >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected - visible_height + 1;
        }
    }

    fn move_selection(&mut self, delta: isize, visible_height: usize) {
        let filtered = self.filtered_indices();
        if filtered.is_empty() {
            return;
        }
        // Find current position in filtered list
        let current_pos = filtered
            .iter()
            .position(|&i| i == self.selected)
            .unwrap_or(0);
        let new_pos =
            (current_pos as isize + delta).clamp(0, (filtered.len() as isize) - 1) as usize;
        self.selected = filtered[new_pos];
        self.auto_scroll = self.selected == filtered[filtered.len() - 1];
        self.ensure_selection_visible(visible_height);
    }

    pub fn handle_event(&mut self, event: Event, area: Rect) {
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                return;
            }
            let visible_height = Self::visible_height(&area);

            // Handle filter input mode
            if self.filter_input.is_some() {
                self.handle_filter_input(key.code, key.modifiers);
                return;
            }

            match self.view_mode {
                ViewMode::List => {
                    self.handle_list_key(key.code, key.modifiers, visible_height);
                }
                ViewMode::Detail => {
                    self.handle_detail_key(key.code, key.modifiers, visible_height);
                }
            }
        }
    }

    fn handle_filter_input(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match code {
            KeyCode::Enter => {
                if let Some(input) = self.filter_input.take() {
                    self.filter = if input.is_empty() { None } else { Some(input) };
                }
                let filtered = self.filtered_indices();
                if !filtered.is_empty() {
                    if self.selected > filtered[filtered.len() - 1] {
                        self.selected = filtered[filtered.len() - 1];
                    } else if !filtered.contains(&self.selected) {
                        self.selected = filtered[0];
                    }
                }
            }
            KeyCode::Esc | KeyCode::Backspace => {
                if let Some(input) = &mut self.filter_input {
                    input.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(input) = &mut self.filter_input {
                    input.push(c);
                }
            }
            _ => {}
        }
    }

    fn handle_list_key(&mut self, code: KeyCode, modifiers: KeyModifiers, visible_height: usize) {
        let filtered = self.filtered_indices();
        let max_idx = filtered.len().saturating_sub(1);

        match (code, modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.handle_ctrl_c();
            }
            (KeyCode::Char('j'), _) | (KeyCode::Down, _) => {
                self.move_selection(1, visible_height);
            }
            (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
                self.move_selection(-1, visible_height);
            }
            (KeyCode::Char('G'), _) if !filtered.is_empty() => {
                self.selected = filtered[max_idx];
                self.auto_scroll = true;
                self.ensure_selection_visible(visible_height);
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                let half = (visible_height / 2).max(1);
                self.move_selection(half as isize, visible_height);
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                let half = (visible_height / 2).max(1);
                self.move_selection(-(half as isize), visible_height);
            }
            (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                self.move_selection(visible_height as isize, visible_height);
            }
            (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                self.move_selection(-(visible_height as isize), visible_height);
            }
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.move_selection(1, visible_height);
            }
            (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
                self.move_selection(-1, visible_height);
            }
            (KeyCode::Enter, _) if !filtered.is_empty() => {
                self.view_mode = ViewMode::Detail;
                self.detail_scroll = 0;
            }
            (KeyCode::Char('/'), _) => {
                self.filter_input = Some(String::new());
            }
            _ => {}
        }
    }

    fn handle_detail_key(&mut self, code: KeyCode, modifiers: KeyModifiers, visible_height: usize) {
        match (code, modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.handle_ctrl_c();
            }
            (KeyCode::Backspace, _) | (KeyCode::Esc, _) => {
                self.view_mode = ViewMode::List;
            }
            (KeyCode::Char('j'), _) | (KeyCode::Down, _) => {
                self.detail_scroll = self.detail_scroll.saturating_add(1);
            }
            (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
                self.detail_scroll = self.detail_scroll.saturating_sub(1);
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                let half = (visible_height / 2).max(1) as u16;
                self.detail_scroll = self.detail_scroll.saturating_add(half);
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                let half = (visible_height / 2).max(1) as u16;
                self.detail_scroll = self.detail_scroll.saturating_sub(half);
            }
            (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                self.detail_scroll = self.detail_scroll.saturating_add(visible_height as u16);
            }
            (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                self.detail_scroll = self.detail_scroll.saturating_sub(visible_height as u16);
            }
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.detail_scroll = self.detail_scroll.saturating_add(1);
            }
            (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
                self.detail_scroll = self.detail_scroll.saturating_sub(1);
            }
            _ => {}
        }
    }

    fn handle_ctrl_c(&mut self) {
        match self.last_ctrl_c {
            Some(last) if last.elapsed() < Duration::from_millis(DOUBLE_CTRL_C_INTERVAL_MS) => {
                self.should_quit = true;
            }
            _ => {
                self.last_ctrl_c = Some(Instant::now());
            }
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        match self.view_mode {
            ViewMode::List => self.render_list(frame, chunks[0]),
            ViewMode::Detail => self.render_detail(frame, chunks[0]),
        }
        self.render_status(frame, chunks[1]);
    }

    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let filtered = self.filtered_indices();
        let width = area.width as usize;
        let visible_height = area.height as usize;

        // Clamp scroll_offset
        let max_offset = filtered.len().saturating_sub(visible_height);
        self.scroll_offset = self.scroll_offset.min(max_offset);

        let visible_start = self.scroll_offset;
        let visible_end = (visible_start + visible_height).min(filtered.len());

        let lines: Vec<Line<'static>> = (visible_start..visible_end)
            .map(|pos| {
                let idx = filtered[pos];
                let line = &self.lines[idx];
                let display = truncate_str(line, width);
                let is_selected = idx == self.selected;

                let spans = highlight_display_line(&display, &self.colors, is_selected);
                if is_selected {
                    // Apply selection highlight to all spans
                    let highlighted: Vec<Span<'static>> = spans
                        .into_iter()
                        .map(|span| {
                            Span::styled(
                                span.content,
                                span.style.patch(
                                    Style::default()
                                        .bg(Color::DarkGray)
                                        .add_modifier(Modifier::BOLD),
                                ),
                            )
                        })
                        .collect();
                    Line::from(highlighted)
                } else {
                    Line::from(spans)
                }
            })
            .collect();

        let text = Text::from(lines);
        let paragraph = Paragraph::new(text);
        frame.render_widget(paragraph, area);
    }

    fn render_detail(&self, frame: &mut Frame, area: Rect) {
        if self.lines.is_empty() || self.selected >= self.lines.len() {
            return;
        }
        let line = &self.lines[self.selected];
        let text = highlight_line(line, &self.colors);

        let paragraph = Paragraph::new(text)
            .scroll((self.detail_scroll, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let status_text = match self.view_mode {
            ViewMode::List => {
                if self.filter_input.is_some() {
                    let input = self.filter_input.as_deref().unwrap_or("");
                    format!(" /{}", input)
                } else {
                    let filter_info = match &self.filter {
                        Some(f) => format!(" [filter:{}] ", f),
                        None => String::new(),
                    };
                    format!(
                        "{}j/k:nav  Enter:detail  /:filter  G:latest  C-d/u/f/b/e/y:scroll  C-c×2:quit",
                        filter_info
                    )
                }
            }
            ViewMode::Detail => "Backspace:back  j/k,C-d/u/f/b/e/y:scroll  C-c×2:quit".to_string(),
        };

        let status = Paragraph::new(Line::from(vec![Span::styled(
            status_text,
            Style::default().fg(Color::White).bg(Color::DarkGray),
        )]));
        frame.render_widget(status, area);
    }

    pub fn poll_events(&self) -> std::io::Result<bool> {
        event::poll(Duration::from_millis(16))
    }

    pub fn next_event(&self) -> std::io::Result<Event> {
        event::read()
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
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

/// Apply lightweight syntax highlighting to a display line for the list view.
fn highlight_display_line(
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
            let end = crate::highlight::find_string_end(rest);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_line() {
        let mut app = App::new(100);
        app.add_line("line1".to_string());
        app.add_line("line2".to_string());
        assert_eq!(app.lines.len(), 2);
    }

    #[test]
    fn test_max_lines_limit() {
        let mut app = App::new(3);
        app.add_line("a".to_string());
        app.add_line("b".to_string());
        app.add_line("c".to_string());
        app.add_line("d".to_string());
        assert_eq!(app.lines, vec!["b", "c", "d"]);
    }

    #[test]
    fn test_max_lines_adjusts_selection() {
        let mut app = App::new(2);
        app.add_line("a".to_string());
        app.add_line("b".to_string());
        // auto_scroll is true, so add_line sets selected to last filtered index
        assert_eq!(app.selected, 1); // Points at "b"
        app.add_line("c".to_string()); // "a" is evicted
        assert_eq!(app.lines, vec!["b", "c"]);
        assert_eq!(app.selected, 1); // Points at "c" (was adjusted + auto_scroll)
    }

    #[test]
    fn test_filter_matching() {
        let mut app = App::new(100);
        app.add_line("{\"name\":\"alice\"}".to_string());
        app.add_line("plain text line".to_string());
        app.add_line("{\"name\":\"bob\"}".to_string());
        app.filter = Some("alice".to_string());
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0]);
    }

    #[test]
    fn test_filter_no_match() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        app.add_line("world".to_string());
        app.filter = Some("xyz".to_string());
        let filtered = app.filtered_indices();
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_clear() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        app.filter = Some("xyz".to_string());
        assert_eq!(app.filtered_indices().len(), 0);
        app.filter = None;
        assert_eq!(app.filtered_indices().len(), 1);
    }

    #[test]
    fn test_move_selection() {
        let mut app = App::new(100);
        app.add_line("a".to_string());
        app.add_line("b".to_string());
        app.add_line("c".to_string());
        app.selected = 0;
        app.move_selection(1, 10);
        assert_eq!(app.selected, 1);
        app.move_selection(1, 10);
        assert_eq!(app.selected, 2);
        app.move_selection(1, 10); // Clamped at end
        assert_eq!(app.selected, 2);
        app.move_selection(-1, 10);
        assert_eq!(app.selected, 1);
        app.move_selection(-1, 10);
        assert_eq!(app.selected, 0);
        app.move_selection(-1, 10); // Clamped at start
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_move_selection_with_filter() {
        let mut app = App::new(100);
        app.add_line("aaa".to_string());
        app.add_line("bbb".to_string());
        app.add_line("aaa2".to_string());
        app.filter = Some("aaa".to_string());
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0, 2]);

        app.selected = 0;
        app.move_selection(1, 10);
        assert_eq!(app.selected, 2); // Skips index 1
    }

    #[test]
    fn test_auto_scroll_on_latest() {
        let mut app = App::new(100);
        app.add_line("a".to_string());
        assert!(app.auto_scroll); // After adding, auto_scroll stays true since we're on latest
    }

    #[test]
    fn test_auto_scroll_off_when_moving_away() {
        let mut app = App::new(100);
        app.add_line("a".to_string());
        app.add_line("b".to_string());
        app.add_line("c".to_string());
        // selected is on last line (2), move up
        app.move_selection(-1, 10);
        assert!(!app.auto_scroll);
    }

    #[test]
    fn test_g_key_jumps_to_latest() {
        let mut app = App::new(100);
        app.add_line("a".to_string());
        app.add_line("b".to_string());
        app.add_line("c".to_string());
        app.selected = 0;
        app.auto_scroll = false;
        // Simulate G key
        let filtered = app.filtered_indices();
        app.selected = filtered[filtered.len() - 1];
        app.auto_scroll = true;
        assert_eq!(app.selected, 2);
        assert!(app.auto_scroll);
    }

    #[test]
    fn test_double_ctrl_c() {
        let mut app = App::new(100);
        app.handle_ctrl_c();
        assert!(!app.should_quit);
        // Second press within interval
        app.handle_ctrl_c();
        assert!(app.should_quit);
    }

    #[test]
    fn test_single_ctrl_c_no_quit() {
        let mut app = App::new(100);
        app.handle_ctrl_c();
        assert!(!app.should_quit);
    }

    #[test]
    fn test_view_mode_toggle() {
        let mut app = App::new(100);
        app.add_line("{\"key\":\"val\"}".to_string());
        assert_eq!(app.view_mode, ViewMode::List);
        app.view_mode = ViewMode::Detail;
        assert_eq!(app.view_mode, ViewMode::Detail);
        app.view_mode = ViewMode::List;
        assert_eq!(app.view_mode, ViewMode::List);
    }

    #[test]
    fn test_vim_scroll_moves_selection() {
        let mut app = App::new(100);
        for i in 0..50 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 10;
        app.scroll_offset = 10;

        // C^d: half page down (visible_height = 10)
        app.move_selection(5, 10); // half of 10
        assert_eq!(app.selected, 15);
    }

    #[test]
    fn test_ensure_selection_visible() {
        let mut app = App::new(100);
        app.selected = 20;
        app.scroll_offset = 0;
        app.ensure_selection_visible(10);
        assert!(app.selected >= app.scroll_offset);
        assert!(app.selected < app.scroll_offset + 10);
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 8), "hello wo…");
    }
}
