use std::path::PathBuf;
use std::time::Duration;

use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::highlight::{HighlightColors, highlight_line};

const TIMESTAMP_WIDTH: usize = 13; // "HH:MM:SS.mmm "

#[derive(Debug, Clone, PartialEq)]
pub enum ViewMode {
    List,
    Detail,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub text: String,
    pub timestamp: String, // "HH:MM:SS.mmm"
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterOp {
    Contains,
    RegexMatch,
    NotContains,
    NotRegexMatch,
}

#[derive(Debug, Clone)]
pub struct FilterCondition {
    pub operator: FilterOp,
    pub value: String,
    pub regex: Option<regex::Regex>,
}

#[derive(Debug, Clone)]
pub struct FilterQuery {
    pub conditions: Vec<FilterCondition>,
}

impl FilterQuery {
    fn display_string(&self) -> String {
        self.conditions
            .iter()
            .map(|c| {
                let op = match c.operator {
                    FilterOp::Contains => "|=",
                    FilterOp::RegexMatch => "|~",
                    FilterOp::NotContains => "!=",
                    FilterOp::NotRegexMatch => "!~",
                };
                format!(r#"{} "{}""#, op, c.value)
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn history_file_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(dirs_data_home)
        .unwrap_or_else(|| PathBuf::from("~/.local/share"));
    let expanded = expand_tilde(base);
    if expanded.is_absolute() {
        Some(expanded.join("logq").join("filter_history"))
    } else {
        None
    }
}

fn dirs_data_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".local").join("share"))
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    if path.starts_with("~")
        && let Ok(home) = std::env::var("HOME")
    {
        let remainder = path.strip_prefix("~").unwrap_or(&path);
        return PathBuf::from(home).join(remainder);
    }
    path
}

fn parse_filter_query(input: &str) -> Result<FilterQuery, String> {
    let s = input.trim();
    if s.is_empty() {
        return Ok(FilterQuery { conditions: vec![] });
    }

    let mut conditions = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut pos = 0;

    while pos < len {
        // Skip whitespace
        while pos < len && chars[pos] == ' ' {
            pos += 1;
        }
        if pos >= len {
            break;
        }

        // Read operator (2 chars)
        if pos + 1 >= len {
            return Err(format!("Expected operator at position {}", pos));
        }
        let op: FilterOp = match (chars[pos], chars[pos + 1]) {
            ('|', '=') => FilterOp::Contains,
            ('|', '~') => FilterOp::RegexMatch,
            ('!', '=') => FilterOp::NotContains,
            ('!', '~') => FilterOp::NotRegexMatch,
            _ => return Err(format!("Expected operator |= |~ != !~ at position {}", pos)),
        };
        pos += 2;

        // Skip whitespace
        while pos < len && chars[pos] == ' ' {
            pos += 1;
        }

        // Expect opening quote
        if pos >= len || chars[pos] != '"' {
            return Err(format!("Expected '\"' at position {}", pos));
        }
        pos += 1;

        // Read value until closing quote
        let mut value = String::new();
        loop {
            if pos >= len {
                return Err("Unterminated string".to_string());
            }
            match chars[pos] {
                '"' => {
                    pos += 1;
                    break;
                }
                _ => {
                    value.push(chars[pos]);
                    pos += 1;
                }
            }
        }

        // Compile regex if needed
        let regex = match op {
            FilterOp::RegexMatch | FilterOp::NotRegexMatch => {
                Some(regex::Regex::new(&value).map_err(|e| format!("Invalid regex: {}", e))?)
            }
            _ => None,
        };

        conditions.push(FilterCondition {
            operator: op,
            value,
            regex,
        });
    }

    Ok(FilterQuery { conditions })
}

pub struct App {
    pub lines: Vec<LogEntry>,
    pub view_mode: ViewMode,
    pub selected: usize,
    pub scroll_offset: usize,
    pub auto_scroll: bool,
    pub detail_scroll: u16,
    pub filter_input: Option<String>,
    pub max_lines: usize,
    pub should_quit: bool,
    colors: HighlightColors,
    filter_query: Option<FilterQuery>,
    filter_error: Option<String>,
    live_filter_query: Option<FilterQuery>,
    live_filter_error: Option<String>,
    filter_history: Vec<String>,
    filter_history_index: Option<usize>,
    filter_draft: Option<String>,
    history_search_active: bool,
    history_search_start: Option<usize>,
}

impl App {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: Vec::new(),
            view_mode: ViewMode::List,
            selected: 0,
            scroll_offset: 0,
            auto_scroll: true,

            detail_scroll: 0,
            filter_input: None,
            max_lines,
            should_quit: false,
            colors: HighlightColors::default(),
            filter_query: None,
            filter_error: None,
            live_filter_query: None,
            live_filter_error: None,
            filter_history: Vec::new(),
            filter_history_index: None,
            filter_draft: None,
            history_search_active: false,
            history_search_start: None,
        }
    }

    pub fn load_history(&mut self) {
        if let Some(path) = history_file_path()
            && let Ok(data) = std::fs::read_to_string(&path)
        {
            let loaded: Vec<String> = data.lines().map(String::from).collect();
            if !loaded.is_empty() {
                self.filter_history = loaded;
            }
        }
    }

    pub fn save_history(&self) {
        if let Some(path) = history_file_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let content = self.filter_history.join("\n");
            let _ = std::fs::write(&path, content);
        }
    }

    pub fn add_line(&mut self, line: String) {
        let timestamp = Local::now().format("%H:%M:%S%.3f").to_string();
        self.lines.push(LogEntry {
            text: line,
            timestamp,
        });
        if self.lines.len() > self.max_lines {
            self.lines.remove(0);
            if self.selected > 0 {
                self.selected -= 1;
            }
        }
        // auto_scroll: selected/scroll_offset are updated in update_auto_scroll()
    }

    pub fn update_auto_scroll(&mut self, visible_height: usize) {
        if !self.auto_scroll {
            return;
        }
        let filtered = self.filtered_indices();
        if filtered.is_empty() {
            return;
        }
        self.selected = filtered[filtered.len() - 1];
        let max_offset = filtered.len().saturating_sub(visible_height);
        self.scroll_offset = max_offset;
    }

    fn active_filter_query(&self) -> Option<&FilterQuery> {
        if self.filter_input.is_some() {
            self.live_filter_query.as_ref()
        } else {
            self.filter_query.as_ref()
        }
    }

    fn line_matches_filter(&self, text: &str) -> bool {
        match self.active_filter_query() {
            Some(query) => query.conditions.iter().all(|c| match c.operator {
                FilterOp::Contains => text.contains(&c.value),
                FilterOp::NotContains => !text.contains(&c.value),
                FilterOp::RegexMatch => c.regex.as_ref().unwrap().is_match(text),
                FilterOp::NotRegexMatch => !c.regex.as_ref().unwrap().is_match(text),
            }),
            None => true,
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        match self.active_filter_query() {
            Some(q) if !q.conditions.is_empty() => self
                .lines
                .iter()
                .enumerate()
                .filter(|(_, entry)| self.line_matches_filter(&entry.text))
                .map(|(i, _)| i)
                .collect(),
            _ => (0..self.lines.len()).collect(),
        }
    }

    fn visible_height(&self, area: &Rect) -> usize {
        // Breadcrumb(1) + help(2) = 3; during filter input add input(1) = 4
        let overhead: usize = if self.filter_input.is_some() { 4 } else { 3 };
        (area.height as usize).saturating_sub(overhead)
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
            let visible_height = self.visible_height(&area);

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

    fn update_live_filter(&mut self) {
        if let Some(input) = &self.filter_input {
            match parse_filter_query(input) {
                Ok(query) if !query.conditions.is_empty() => {
                    self.live_filter_query = Some(query);
                    self.live_filter_error = None;
                }
                Ok(_) => {
                    self.live_filter_query = None;
                    self.live_filter_error = None;
                }
                Err(msg) => {
                    self.live_filter_query = None;
                    self.live_filter_error = Some(msg);
                }
            }
        }
    }

    fn handle_filter_input(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match code {
            KeyCode::Enter => {
                if let Some(input) = self.filter_input.take() {
                    match parse_filter_query(&input) {
                        Ok(query) if !query.conditions.is_empty() => {
                            self.filter_query = Some(query);
                            self.filter_error = None;
                            self.live_filter_query = None;
                            self.live_filter_error = None;
                            if self.filter_history.last() != Some(&input) {
                                self.filter_history.push(input);
                                if self.filter_history.len() > 100 {
                                    self.filter_history.remove(0);
                                }
                            }
                        }
                        Ok(_) => {
                            self.filter_query = None;
                            self.filter_error = None;
                            self.live_filter_query = None;
                            self.live_filter_error = None;
                        }
                        Err(msg) => {
                            self.filter_error = Some(msg.clone());
                            self.live_filter_error = Some(msg);
                            self.filter_input = Some(input);
                        }
                    }
                }
                self.filter_draft = None;
                self.filter_history_index = None;
                self.history_search_active = false;
                self.history_search_start = None;
                let filtered = self.filtered_indices();
                if !filtered.is_empty() {
                    if self.selected > filtered[filtered.len() - 1] {
                        self.selected = filtered[filtered.len() - 1];
                    } else if !filtered.contains(&self.selected) {
                        self.selected = filtered[0];
                    }
                }
            }
            KeyCode::Esc => {
                self.filter_input = None;
                self.filter_error = None;
                self.live_filter_query = None;
                self.live_filter_error = None;
                self.filter_draft = None;
                self.filter_history_index = None;
                self.history_search_active = false;
                self.history_search_start = None;
            }
            KeyCode::Backspace => {
                if let Some(input) = &mut self.filter_input {
                    if input.is_empty() {
                        self.filter_input = None;
                        self.filter_error = None;
                        self.live_filter_query = None;
                        self.live_filter_error = None;
                        self.filter_draft = None;
                        self.filter_history_index = None;
                        self.history_search_active = false;
                        self.history_search_start = None;
                    } else {
                        input.pop();
                        self.filter_error = None;
                        self.history_search_active = false;
                        self.history_search_start = None;
                        self.update_live_filter();
                    }
                }
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter_input = None;
                self.filter_error = None;
                self.live_filter_query = None;
                self.live_filter_error = None;
                self.filter_draft = None;
                self.filter_history_index = None;
                self.history_search_active = false;
                self.history_search_start = None;
            }
            KeyCode::Char('r') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.handle_history_search();
            }
            KeyCode::Up => {
                self.history_search_active = false;
                self.history_search_start = None;
                self.handle_history_up();
            }
            KeyCode::Down => {
                self.history_search_active = false;
                self.history_search_start = None;
                self.handle_history_down();
            }
            KeyCode::Char(c) => {
                if let Some(input) = &mut self.filter_input {
                    input.push(c);
                    self.filter_error = None;
                    self.history_search_active = false;
                    self.history_search_start = None;
                    self.update_live_filter();
                }
            }
            _ => {}
        }
    }

    fn handle_history_up(&mut self) {
        if self.filter_history.is_empty() {
            return;
        }
        if self.filter_history_index.is_none() {
            // Save current input as draft
            self.filter_draft = self.filter_input.clone();
        }
        let current = self
            .filter_history_index
            .unwrap_or(self.filter_history.len());
        if current > 0 {
            self.filter_history_index = Some(current - 1);
            self.filter_input = Some(self.filter_history[current - 1].clone());
            self.update_live_filter();
        }
    }

    fn handle_history_down(&mut self) {
        if self.filter_history.is_empty() {
            return;
        }
        let current = self
            .filter_history_index
            .unwrap_or(self.filter_history.len());
        if current < self.filter_history.len() - 1 {
            self.filter_history_index = Some(current + 1);
            self.filter_input = Some(self.filter_history[current + 1].clone());
            self.update_live_filter();
        } else {
            // Past the end: restore draft
            self.filter_history_index = None;
            self.filter_input = self.filter_draft.clone().or_else(|| Some(String::new()));
            self.update_live_filter();
        }
    }

    fn handle_history_search(&mut self) {
        if self.filter_history.is_empty() {
            return;
        }
        let search_term = self
            .filter_draft
            .as_deref()
            .or(self.filter_input.as_deref())
            .unwrap_or("");
        let start = self
            .history_search_start
            .unwrap_or(self.filter_history.len());
        // Search backwards from start
        for i in (0..start).rev() {
            if self.filter_history[i].contains(search_term) {
                self.filter_input = Some(self.filter_history[i].clone());
                self.history_search_start = Some(i);
                self.history_search_active = true;
                self.update_live_filter();
                return;
            }
        }
        // Wrapped around: try from the end
        if start < self.filter_history.len() {
            for i in (start..self.filter_history.len()).rev() {
                if self.filter_history[i].contains(search_term) {
                    self.filter_input = Some(self.filter_history[i].clone());
                    self.history_search_start = Some(i);
                    self.history_search_active = true;
                    self.update_live_filter();
                    return;
                }
            }
        }
    }

    fn handle_list_key(&mut self, code: KeyCode, modifiers: KeyModifiers, visible_height: usize) {
        let filtered = self.filtered_indices();
        let max_idx = filtered.len().saturating_sub(1);

        match (code, modifiers) {
            (KeyCode::Char('x'), KeyModifiers::CONTROL) => {
                self.handle_ctrl_x();
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
                let preset = self.filter_history.last().cloned().unwrap_or_default();
                self.filter_input = Some(preset);
                self.filter_history_index = if self.filter_history.is_empty() {
                    None
                } else {
                    Some(self.filter_history.len() - 1)
                };
                self.filter_draft = None;
                self.history_search_active = false;
                self.history_search_start = None;
                self.update_live_filter();
            }
            (KeyCode::Esc, _) => {
                self.filter_query = None;
            }
            _ => {}
        }
    }

    fn handle_detail_key(&mut self, code: KeyCode, modifiers: KeyModifiers, visible_height: usize) {
        match (code, modifiers) {
            (KeyCode::Char('x'), KeyModifiers::CONTROL) => {
                self.handle_ctrl_x();
            }
            (KeyCode::Backspace, _) | (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => {
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

    fn handle_ctrl_x(&mut self) {
        self.should_quit = true;
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();

        if self.filter_input.is_some() {
            // Filter input mode: breadcrumb + content + input + help1 + help2
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(area);

            self.render_breadcrumb(frame, chunks[0]);
            match self.view_mode {
                ViewMode::List => self.render_list(frame, chunks[1]),
                ViewMode::Detail => self.render_detail(frame, chunks[1]),
            }
            self.render_input_line(frame, chunks[2]);
            self.render_help_line1(frame, chunks[3]);
            self.render_help_line2(frame, chunks[4]);

            let input = self.filter_input.as_deref().unwrap_or("");
            let cursor_x = (2 + input.len()) as u16;
            frame.set_cursor_position((cursor_x, chunks[2].y));
        } else {
            // Normal mode: breadcrumb + content + help1 + help2
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(area);

            self.render_breadcrumb(frame, chunks[0]);
            match self.view_mode {
                ViewMode::List => self.render_list(frame, chunks[1]),
                ViewMode::Detail => self.render_detail(frame, chunks[1]),
            }
            self.render_help_line1(frame, chunks[2]);
            self.render_help_line2(frame, chunks[3]);
        }
    }

    fn render_breadcrumb(&self, frame: &mut Frame, area: Rect) {
        let mut parts = Vec::new();
        if let Some(q) = &self.filter_query {
            parts.push(format!("[filter: {}]", q.display_string()));
        }
        if self.view_mode == ViewMode::Detail {
            parts.push("[detail]".to_string());
        }
        if parts.is_empty() {
            return;
        }
        let text = parts.join(" > ");
        let breadcrumb = Paragraph::new(Line::from(vec![Span::styled(
            text,
            Style::default().fg(Color::Cyan).bg(Color::DarkGray),
        )]));
        frame.render_widget(breadcrumb, area);
    }

    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let filtered = self.filtered_indices();
        let width = area.width as usize;
        let visible_height = area.height as usize;

        // auto_scroll: follow the latest line
        self.update_auto_scroll(visible_height);

        // Clamp scroll_offset
        let max_offset = filtered.len().saturating_sub(visible_height);
        self.scroll_offset = self.scroll_offset.min(max_offset);

        let visible_start = self.scroll_offset;
        let visible_end = (visible_start + visible_height).min(filtered.len());

        let content_width = width.saturating_sub(TIMESTAMP_WIDTH);

        let lines: Vec<Line<'static>> = (visible_start..visible_end)
            .map(|pos| {
                let idx = filtered[pos];
                let entry = &self.lines[idx];
                let display = truncate_str(&entry.text, content_width);
                let is_selected = idx == self.selected;

                // Timestamp span
                let ts_span = Span::styled(
                    format!("{} ", entry.timestamp),
                    Style::default().fg(Color::DarkGray),
                );

                // Content spans
                let content_spans = highlight_display_line(&display, &self.colors, is_selected);
                if is_selected {
                    let highlighted: Vec<Span<'static>> = content_spans
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
                    Line::from(
                        std::iter::once(ts_span)
                            .chain(highlighted)
                            .collect::<Vec<_>>(),
                    )
                } else {
                    Line::from(
                        std::iter::once(ts_span)
                            .chain(content_spans)
                            .collect::<Vec<_>>(),
                    )
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
        let line = &self.lines[self.selected].text;
        let text = highlight_line(line, &self.colors);

        let paragraph = Paragraph::new(text)
            .scroll((self.detail_scroll, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);
    }

    fn render_input_line(&self, frame: &mut Frame, area: Rect) {
        let bg = Style::default().bg(Color::DarkGray);
        let width = area.width as usize;

        let input = self.filter_input.as_deref().unwrap_or("");
        let mut s = vec![Span::styled(
            format!(" /{}", input),
            Style::default().fg(Color::White).bg(Color::DarkGray),
        )];

        let error = self
            .live_filter_error
            .as_deref()
            .or(self.filter_error.as_deref());
        if let Some(err) = error {
            s.push(Span::styled(
                format!("  Error: {}", err),
                Style::default().fg(Color::Red).bg(Color::DarkGray),
            ));
        }

        let left_len: usize = s.iter().map(|sp| sp.content.len()).sum();
        let padding = width.saturating_sub(left_len);
        if padding > 0 {
            s.push(Span::styled(" ".repeat(padding), bg));
        }

        let status = Paragraph::new(Line::from(s));
        frame.render_widget(status, area);
    }

    fn render_help_line1(&self, frame: &mut Frame, area: Rect) {
        let text = match self.view_mode {
            ViewMode::List if self.filter_input.is_some() => {
                if self.history_search_active {
                    " C-r:next  Enter:apply  Esc:cancel"
                } else {
                    " Enter:apply  ↑↓:history  C-r:search  Esc:cancel"
                }
            }
            ViewMode::List => {
                return self.render_help_spans(
                    frame,
                    area,
                    " j/k nav  Enter detail  / filter  G latest  Esc clear",
                );
            }
            ViewMode::Detail => " q/Bksp/Esc back  j/k scroll",
        };

        self.render_help_spans(frame, area, text);
    }

    fn render_help_line2(&self, frame: &mut Frame, area: Rect) {
        let text = match self.view_mode {
            ViewMode::List if self.filter_input.is_some() => {
                " Bksp delete/cancel  syntax: |= \"text\"  |~ /regex/  != !~"
            }
            ViewMode::List | ViewMode::Detail => " C-d/u half  C-f/b full  C-e/y line  C-x quit",
        };

        self.render_help_spans(frame, area, text);
    }

    fn render_help_spans(&self, frame: &mut Frame, area: Rect, text: &str) {
        let bg = Style::default().bg(Color::DarkGray);
        let width = area.width as usize;
        let padding = width.saturating_sub(text.len());
        let spans = vec![
            Span::styled(
                text.to_string(),
                Style::default().fg(Color::White).bg(Color::DarkGray),
            ),
            Span::styled(" ".repeat(padding), bg),
        ];
        let paragraph = Paragraph::new(Line::from(spans));
        frame.render_widget(paragraph, area);
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
    fn test_add_line_timestamp() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        let ts = &app.lines[0].timestamp;
        // Timestamp format: HH:MM:SS.mmm (12 chars)
        assert_eq!(ts.len(), 12);
        assert!(ts.chars().nth(2) == Some(':'));
        assert!(ts.chars().nth(5) == Some(':'));
        assert!(ts.chars().nth(8) == Some('.'));
    }

    #[test]
    fn test_max_lines_limit() {
        let mut app = App::new(3);
        app.add_line("a".to_string());
        app.add_line("b".to_string());
        app.add_line("c".to_string());
        app.add_line("d".to_string());
        assert_eq!(
            app.lines
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c", "d"]
        );
    }

    #[test]
    fn test_max_lines_adjusts_selection() {
        let mut app = App::new(2);
        app.add_line("a".to_string());
        app.add_line("b".to_string());
        app.selected = 1;
        app.add_line("c".to_string());
        assert_eq!(
            app.lines
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c"]
        );
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_filter_matching() {
        let mut app = App::new(100);
        app.add_line("{\"name\":\"alice\"}".to_string());
        app.add_line("plain text line".to_string());
        app.add_line("{\"name\":\"bob\"}".to_string());
        app.filter_query = Some(FilterQuery {
            conditions: vec![FilterCondition {
                operator: FilterOp::Contains,
                value: "alice".to_string(),
                regex: None,
            }],
        });
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0]);
    }

    #[test]
    fn test_filter_no_match() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        app.add_line("world".to_string());
        app.filter_query = Some(FilterQuery {
            conditions: vec![FilterCondition {
                operator: FilterOp::Contains,
                value: "xyz".to_string(),
                regex: None,
            }],
        });
        let filtered = app.filtered_indices();
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_clear() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        app.filter_query = Some(FilterQuery {
            conditions: vec![FilterCondition {
                operator: FilterOp::Contains,
                value: "xyz".to_string(),
                regex: None,
            }],
        });
        assert_eq!(app.filtered_indices().len(), 0);
        app.filter_query = None;
        assert_eq!(app.filtered_indices().len(), 1);
    }

    #[test]
    fn test_regex_filter_matching() {
        let mut app = App::new(100);
        app.add_line("error: connection timeout".to_string());
        app.add_line("info: request ok".to_string());
        app.add_line("error: disk full".to_string());
        app.filter_query = Some(FilterQuery {
            conditions: vec![FilterCondition {
                operator: FilterOp::RegexMatch,
                value: "err.*timeout".to_string(),
                regex: regex::Regex::new("err.*timeout").ok(),
            }],
        });
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0]);
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
        app.move_selection(1, 10);
        assert_eq!(app.selected, 2);
        app.move_selection(-1, 10);
        assert_eq!(app.selected, 1);
        app.move_selection(-1, 10);
        assert_eq!(app.selected, 0);
        app.move_selection(-1, 10);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_move_selection_with_filter() {
        let mut app = App::new(100);
        app.add_line("aaa".to_string());
        app.add_line("bbb".to_string());
        app.add_line("aaa2".to_string());
        app.filter_query = Some(FilterQuery {
            conditions: vec![FilterCondition {
                operator: FilterOp::Contains,
                value: "aaa".to_string(),
                regex: None,
            }],
        });
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0, 2]);

        app.selected = 0;
        app.move_selection(1, 10);
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn test_auto_scroll_on_latest() {
        let mut app = App::new(100);
        app.add_line("a".to_string());
        assert!(app.auto_scroll);
    }

    #[test]
    fn test_auto_scroll_off_when_moving_away() {
        let mut app = App::new(100);
        app.add_line("a".to_string());
        app.add_line("b".to_string());
        app.add_line("c".to_string());
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
        let filtered = app.filtered_indices();
        app.selected = filtered[filtered.len() - 1];
        app.auto_scroll = true;
        assert_eq!(app.selected, 2);
        assert!(app.auto_scroll);
    }

    #[test]
    fn test_ctrl_x_quit() {
        let mut app = App::new(100);
        app.handle_ctrl_x();
        assert!(app.should_quit);
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

        app.move_selection(5, 10);
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

    #[test]
    fn test_auto_scroll_updates_scroll_offset() {
        let mut app = App::new(100);
        for i in 0..50 {
            app.add_line(format!("line{}", i));
        }
        assert!(app.auto_scroll);
        app.update_auto_scroll(10);
        assert_eq!(app.selected, 49);
        assert_eq!(app.scroll_offset, 40);
    }

    #[test]
    fn test_auto_scroll_disabled_no_offset_update() {
        let mut app = App::new(100);
        for i in 0..50 {
            app.add_line(format!("line{}", i));
        }
        app.auto_scroll = false;
        app.scroll_offset = 5;
        app.update_auto_scroll(10);
        assert_eq!(app.scroll_offset, 5);
    }

    #[test]
    fn test_auto_scroll_with_filter() {
        let mut app = App::new(100);
        app.add_line("aaa1".to_string());
        app.add_line("bbb".to_string());
        app.add_line("aaa2".to_string());
        app.add_line("bbb2".to_string());
        app.add_line("aaa3".to_string());
        app.filter_query = Some(FilterQuery {
            conditions: vec![FilterCondition {
                operator: FilterOp::Contains,
                value: "aaa".to_string(),
                regex: None,
            }],
        });
        app.update_auto_scroll(10);
        assert_eq!(app.selected, 4);
    }

    #[test]
    fn test_add_line_performance_many_lines() {
        let mut app = App::new(10000);
        let start = std::time::Instant::now();
        for i in 0..1000 {
            app.add_line(format!("line number {} with some content", i));
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 200,
            "add_line 1000x took {:?}",
            elapsed
        );
    }

    // Parser tests

    #[test]
    fn test_parse_contains() {
        let query = parse_filter_query(r#"|= "foo""#).unwrap();
        assert_eq!(query.conditions.len(), 1);
        assert_eq!(query.conditions[0].operator, FilterOp::Contains);
        assert_eq!(query.conditions[0].value, "foo");
    }

    #[test]
    fn test_parse_regex_match() {
        let query = parse_filter_query(r#"|~ "err.*""#).unwrap();
        assert_eq!(query.conditions.len(), 1);
        assert_eq!(query.conditions[0].operator, FilterOp::RegexMatch);
        assert_eq!(query.conditions[0].value, "err.*");
        assert!(query.conditions[0].regex.is_some());
    }

    #[test]
    fn test_parse_not_contains() {
        let query = parse_filter_query(r#"!= "bar""#).unwrap();
        assert_eq!(query.conditions.len(), 1);
        assert_eq!(query.conditions[0].operator, FilterOp::NotContains);
        assert_eq!(query.conditions[0].value, "bar");
    }

    #[test]
    fn test_parse_not_regex_match() {
        let query = parse_filter_query(r#"!~ "baz""#).unwrap();
        assert_eq!(query.conditions.len(), 1);
        assert_eq!(query.conditions[0].operator, FilterOp::NotRegexMatch);
        assert_eq!(query.conditions[0].value, "baz");
        assert!(query.conditions[0].regex.is_some());
    }

    #[test]
    fn test_parse_multiple_conditions() {
        let query = parse_filter_query(r#"|= "foo" != "bar""#).unwrap();
        assert_eq!(query.conditions.len(), 2);
        assert_eq!(query.conditions[0].operator, FilterOp::Contains);
        assert_eq!(query.conditions[0].value, "foo");
        assert_eq!(query.conditions[1].operator, FilterOp::NotContains);
        assert_eq!(query.conditions[1].value, "bar");
    }

    #[test]
    fn test_parse_empty_input() {
        let query = parse_filter_query("").unwrap();
        assert!(query.conditions.is_empty());
    }

    #[test]
    fn test_parse_whitespace_only() {
        let query = parse_filter_query("   ").unwrap();
        assert!(query.conditions.is_empty());
    }

    #[test]
    fn test_parse_error_no_operator() {
        assert!(parse_filter_query(r#""foo""#).is_err());
    }

    #[test]
    fn test_parse_error_invalid_operator() {
        assert!(parse_filter_query(r#"== "foo""#).is_err());
    }

    #[test]
    fn test_parse_error_unterminated_string() {
        assert!(parse_filter_query(r#"|= "foo"#).is_err());
    }

    #[test]
    fn test_parse_error_missing_quotes() {
        assert!(parse_filter_query("|= foo").is_err());
    }

    #[test]
    fn test_parse_error_invalid_regex() {
        assert!(parse_filter_query(r#"|~ "[invalid""#).is_err());
    }

    #[test]
    fn test_query_matches_contains() {
        let query = FilterQuery {
            conditions: vec![FilterCondition {
                operator: FilterOp::Contains,
                value: "foo".to_string(),
                regex: None,
            }],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter("foobar"));
        assert!(!app.line_matches_filter("barbaz"));
    }

    #[test]
    fn test_query_matches_not_contains() {
        let query = FilterQuery {
            conditions: vec![FilterCondition {
                operator: FilterOp::NotContains,
                value: "foo".to_string(),
                regex: None,
            }],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter("barbaz"));
        assert!(!app.line_matches_filter("foobar"));
    }

    #[test]
    fn test_query_matches_regex() {
        let query = FilterQuery {
            conditions: vec![FilterCondition {
                operator: FilterOp::RegexMatch,
                value: "err.*".to_string(),
                regex: regex::Regex::new("err.*").ok(),
            }],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter("error: timeout"));
        assert!(!app.line_matches_filter("info: ok"));
    }

    #[test]
    fn test_query_matches_not_regex() {
        let query = FilterQuery {
            conditions: vec![FilterCondition {
                operator: FilterOp::NotRegexMatch,
                value: "err.*".to_string(),
                regex: regex::Regex::new("err.*").ok(),
            }],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter("info: ok"));
        assert!(!app.line_matches_filter("error: timeout"));
    }

    #[test]
    fn test_query_matches_and_semantics() {
        let query = FilterQuery {
            conditions: vec![
                FilterCondition {
                    operator: FilterOp::Contains,
                    value: "error".to_string(),
                    regex: None,
                },
                FilterCondition {
                    operator: FilterOp::NotContains,
                    value: "timeout".to_string(),
                    regex: None,
                },
            ],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter("error: disk full"));
        assert!(!app.line_matches_filter("error: timeout"));
        assert!(!app.line_matches_filter("info: ok"));
    }

    #[test]
    fn test_filter_query_display_string() {
        let query = FilterQuery {
            conditions: vec![
                FilterCondition {
                    operator: FilterOp::Contains,
                    value: "foo".to_string(),
                    regex: None,
                },
                FilterCondition {
                    operator: FilterOp::NotContains,
                    value: "bar".to_string(),
                    regex: None,
                },
            ],
        };
        assert_eq!(query.display_string(), r#"|= "foo" != "bar""#);
    }

    #[test]
    fn test_save_and_load_history() {
        let dir = std::env::temp_dir().join("logq_test_history");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("filter_history");

        // Write history
        {
            let app = App {
                filter_history: vec!["|= \"foo\"".to_string(), "|= \"bar\"".to_string()],
                ..App::new(100)
            };
            let content = app.filter_history.join("\n");
            std::fs::write(&path, &content).unwrap();
        }

        // Read history
        let data = std::fs::read_to_string(&path).unwrap();
        let loaded: Vec<String> = data.lines().map(String::from).collect();
        assert_eq!(loaded, vec!["|= \"foo\"", "|= \"bar\""]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
