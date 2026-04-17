use std::path::PathBuf;
use std::time::Duration;

use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use serde_json::Value;

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
pub enum FilterValue {
    String(String),
    Number(f64),
    Boolean(bool),
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterOp {
    Contains,
    RegexMatch,
    NotContains,
    NotRegexMatch,
    JsonEquals,
    JsonNotEquals,
    JsonRegexMatch,
    JsonNotRegexMatch,
}

#[derive(Debug, Clone)]
pub struct FilterCondition {
    pub operator: FilterOp,
    pub value: FilterValue,
    pub regex: Option<regex::Regex>,
    pub json_key: Option<String>,
}

#[derive(Debug, Clone)]
pub enum JsonExpr {
    Condition(FilterCondition),
    And(Box<JsonExpr>, Box<JsonExpr>),
    Or(Box<JsonExpr>, Box<JsonExpr>),
}

#[derive(Debug, Clone)]
pub enum FilterSegment {
    Plain(FilterCondition),
    Json(JsonExpr),
}

#[derive(Debug, Clone)]
pub struct FilterQuery {
    pub segments: Vec<FilterSegment>,
}

impl FilterValue {
    fn display_string(&self) -> String {
        match self {
            FilterValue::String(s) => format!("\"{}\"", s),
            FilterValue::Number(n) => {
                if n.fract() == 0.0 {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            FilterValue::Boolean(b) => b.to_string(),
            FilterValue::Null => "null".to_string(),
        }
    }
}

impl FilterQuery {
    fn display_string(&self) -> String {
        self.segments
            .iter()
            .map(|seg| match seg {
                FilterSegment::Plain(c) => {
                    let op = match c.operator {
                        FilterOp::Contains => "|=",
                        FilterOp::RegexMatch => "|~",
                        FilterOp::NotContains => "!=",
                        FilterOp::NotRegexMatch => "!~",
                        _ => unreachable!(),
                    };
                    format!("{} {}", op, c.value.display_string())
                }
                FilterSegment::Json(expr) => {
                    format!("| {}", expr.display_string_inner(false))
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl JsonExpr {
    fn display_string_inner(&self, parent_is_and: bool) -> String {
        match self {
            JsonExpr::Condition(c) => {
                let op = match c.operator {
                    FilterOp::JsonEquals => "=",
                    FilterOp::JsonNotEquals => "!=",
                    FilterOp::JsonRegexMatch => "=~",
                    FilterOp::JsonNotRegexMatch => "!~",
                    _ => unreachable!(),
                };
                format!(
                    "{} {} {}",
                    c.json_key.as_deref().unwrap(),
                    op,
                    c.value.display_string()
                )
            }
            JsonExpr::And(l, r) => {
                format!(
                    "{} and {}",
                    l.display_string_inner(true),
                    r.display_string_inner(true)
                )
            }
            JsonExpr::Or(l, r) => {
                let inner = format!(
                    "{} or {}",
                    l.display_string_inner(false),
                    r.display_string_inner(false)
                );
                if parent_is_and {
                    format!("({})", inner)
                } else {
                    inner
                }
            }
        }
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

fn lookup_json_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in key.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn compare_json_value(actual: &Value, expected: &FilterValue) -> bool {
    match (actual, expected) {
        (Value::String(s), FilterValue::String(e)) => s == e,
        (Value::Number(n), FilterValue::Number(e)) => n.as_f64() == Some(*e),
        (Value::Bool(b), FilterValue::Boolean(e)) => b == e,
        (Value::Null, FilterValue::Null) => true,
        _ => false,
    }
}

fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

fn skip_whitespace(chars: &[char], pos: &mut usize, len: usize) {
    while *pos < len && chars[*pos] == ' ' {
        *pos += 1;
    }
}

fn is_keyword(chars: &[char], pos: usize, len: usize, keyword: &str) -> bool {
    let kw_chars: Vec<char> = keyword.chars().collect();
    let kw_len = kw_chars.len();
    if pos + kw_len > len {
        return false;
    }
    if chars[pos..pos + kw_len] != kw_chars[..] {
        return false;
    }
    // Word boundary: followed by space, '(', ')', or end of input
    if pos + kw_len < len {
        let next = chars[pos + kw_len];
        next == ' ' || next == '(' || next == ')'
    } else {
        true
    }
}

fn parse_json_condition(
    chars: &[char],
    pos: &mut usize,
    len: usize,
) -> Result<FilterCondition, String> {
    // Read key name
    let key_start = *pos;
    while *pos < len
        && (chars[*pos].is_alphanumeric()
            || chars[*pos] == '_'
            || chars[*pos] == '-'
            || chars[*pos] == '.')
    {
        *pos += 1;
    }
    let key: String = chars[key_start..*pos].iter().collect();
    if key.is_empty() {
        return Err(format!("Expected key name at position {}", pos));
    }

    skip_whitespace(chars, pos, len);

    // Read operator: =, !=, =~, !~
    let (op, needs_regex) = if *pos + 1 < len && chars[*pos] == '!' {
        if chars[*pos + 1] == '=' {
            *pos += 2;
            (FilterOp::JsonNotEquals, false)
        } else if chars[*pos + 1] == '~' {
            *pos += 2;
            (FilterOp::JsonNotRegexMatch, true)
        } else {
            return Err(format!("Expected operator = != =~ !~ at position {}", pos));
        }
    } else if *pos < len && chars[*pos] == '=' {
        *pos += 1;
        if *pos < len && chars[*pos] == '~' {
            *pos += 1;
            (FilterOp::JsonRegexMatch, true)
        } else {
            (FilterOp::JsonEquals, false)
        }
    } else {
        return Err(format!("Expected operator = != =~ !~ at position {}", pos));
    };

    skip_whitespace(chars, pos, len);

    // Read value
    let (value, regex) = if *pos < len && chars[*pos] == '"' {
        let s = parse_quoted_string(chars, pos, len)?;
        let r = if needs_regex {
            Some(regex::Regex::new(&s).map_err(|e| format!("Invalid regex: {}", e))?)
        } else {
            None
        };
        (FilterValue::String(s), r)
    } else if *pos + 3 < len && chars[*pos..*pos + 4] == ['t', 'r', 'u', 'e'] {
        *pos += 4;
        (FilterValue::Boolean(true), None)
    } else if *pos + 4 < len && chars[*pos..*pos + 5] == ['f', 'a', 'l', 's', 'e'] {
        *pos += 5;
        (FilterValue::Boolean(false), None)
    } else if *pos + 3 < len && chars[*pos..*pos + 4] == ['n', 'u', 'l', 'l'] {
        *pos += 4;
        (FilterValue::Null, None)
    } else {
        // Number
        let num_start = *pos;
        if *pos < len && (chars[*pos] == '-' || chars[*pos] == '+') {
            *pos += 1;
        }
        let has_digits = {
            let start = *pos;
            while *pos < len && chars[*pos].is_ascii_digit() {
                *pos += 1;
            }
            *pos > start
        };
        if !has_digits {
            return Err(format!("Expected value at position {}", num_start));
        }
        if *pos < len && chars[*pos] == '.' {
            *pos += 1;
            while *pos < len && chars[*pos].is_ascii_digit() {
                *pos += 1;
            }
        }
        let num_str: String = chars[num_start..*pos].iter().collect();
        let n: f64 = num_str
            .parse()
            .map_err(|_| format!("Invalid number: {}", num_str))?;
        (FilterValue::Number(n), None)
    };

    Ok(FilterCondition {
        operator: op,
        value,
        regex,
        json_key: Some(key),
    })
}

fn parse_json_primary(chars: &[char], pos: &mut usize, len: usize) -> Result<JsonExpr, String> {
    skip_whitespace(chars, pos, len);

    if *pos < len && chars[*pos] == '(' {
        *pos += 1; // skip '('
        let expr = parse_json_or_expr(chars, pos, len)?;
        skip_whitespace(chars, pos, len);
        if *pos >= len || chars[*pos] != ')' {
            return Err("Expected ')'".to_string());
        }
        *pos += 1; // skip ')'
        Ok(expr)
    } else {
        let cond = parse_json_condition(chars, pos, len)?;
        Ok(JsonExpr::Condition(cond))
    }
}

fn parse_json_and_expr(chars: &[char], pos: &mut usize, len: usize) -> Result<JsonExpr, String> {
    let mut left = parse_json_primary(chars, pos, len)?;

    loop {
        skip_whitespace(chars, pos, len);
        if is_keyword(chars, *pos, len, "and") {
            *pos += 3; // consume "and"
            let right = parse_json_primary(chars, pos, len)?;
            left = JsonExpr::And(Box::new(left), Box::new(right));
        } else {
            break;
        }
    }

    Ok(left)
}

fn parse_json_or_expr(chars: &[char], pos: &mut usize, len: usize) -> Result<JsonExpr, String> {
    let mut left = parse_json_and_expr(chars, pos, len)?;

    loop {
        skip_whitespace(chars, pos, len);
        if is_keyword(chars, *pos, len, "or") {
            *pos += 2; // consume "or"
            let right = parse_json_and_expr(chars, pos, len)?;
            left = JsonExpr::Or(Box::new(left), Box::new(right));
        } else {
            break;
        }
    }

    Ok(left)
}

fn parse_filter_query(input: &str) -> Result<FilterQuery, String> {
    let s = input.trim();
    if s.is_empty() {
        return Ok(FilterQuery { segments: vec![] });
    }

    let mut segments: Vec<FilterSegment> = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut pos = 0;

    while pos < len {
        skip_whitespace(&chars, &mut pos, len);
        if pos >= len {
            break;
        }

        if chars[pos] == '|' {
            if pos + 1 < len && (chars[pos + 1] == '=' || chars[pos + 1] == '~') {
                // Plain text operators: |= or |~
                let op: FilterOp = match chars[pos + 1] {
                    '=' => FilterOp::Contains,
                    '~' => FilterOp::RegexMatch,
                    _ => unreachable!(),
                };
                pos += 2;

                skip_whitespace(&chars, &mut pos, len);

                let value = parse_quoted_string(&chars, &mut pos, len)?;
                let regex = match op {
                    FilterOp::RegexMatch => Some(
                        regex::Regex::new(&value).map_err(|e| format!("Invalid regex: {}", e))?,
                    ),
                    _ => None,
                };

                segments.push(FilterSegment::Plain(FilterCondition {
                    operator: op,
                    value: FilterValue::String(value),
                    regex,
                    json_key: None,
                }));
            } else {
                // JSON key group: | followed by expression with and/or/parens
                pos += 1; // skip '|'
                skip_whitespace(&chars, &mut pos, len);

                let expr = parse_json_or_expr(&chars, &mut pos, len)?;
                segments.push(FilterSegment::Json(expr));
            }
        } else if chars[pos] == '!' {
            // Plain text operators: != or !~
            if pos + 1 >= len {
                return Err(format!("Expected operator at position {}", pos));
            }
            let op: FilterOp = match chars[pos + 1] {
                '=' => FilterOp::NotContains,
                '~' => FilterOp::NotRegexMatch,
                _ => return Err(format!("Expected operator |= |~ != !~ at position {}", pos)),
            };
            pos += 2;

            skip_whitespace(&chars, &mut pos, len);

            let value = parse_quoted_string(&chars, &mut pos, len)?;
            let regex = match op {
                FilterOp::NotRegexMatch => {
                    Some(regex::Regex::new(&value).map_err(|e| format!("Invalid regex: {}", e))?)
                }
                _ => None,
            };

            segments.push(FilterSegment::Plain(FilterCondition {
                operator: op,
                value: FilterValue::String(value),
                regex,
                json_key: None,
            }));
        } else {
            return Err(format!(
                "Expected operator |= |~ != !~ or | key at position {}",
                pos
            ));
        }
    }

    Ok(FilterQuery { segments })
}

fn parse_quoted_string(chars: &[char], pos: &mut usize, len: usize) -> Result<String, String> {
    if *pos >= len || chars[*pos] != '"' {
        return Err(format!("Expected '\"' at position {}", pos));
    }
    *pos += 1;
    let mut value = String::new();
    loop {
        if *pos >= len {
            return Err("Unterminated string".to_string());
        }
        match chars[*pos] {
            '\\' => {
                *pos += 1;
                if *pos >= len {
                    return Err("Unterminated escape".to_string());
                }
                value.push('\\');
                value.push(chars[*pos]);
                *pos += 1;
            }
            '"' => {
                *pos += 1;
                break;
            }
            _ => {
                value.push(chars[*pos]);
                *pos += 1;
            }
        }
    }
    Ok(value)
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
    filtered_indices_cache: Option<Vec<usize>>,
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
            filtered_indices_cache: None,
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
        self.filtered_indices_cache = None;
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
            Some(query) => query.segments.iter().all(|seg| match seg {
                FilterSegment::Plain(c) => self.plain_condition_matches(text, c),
                FilterSegment::Json(expr) => self.json_expr_matches(text, expr),
            }),
            None => true,
        }
    }

    fn plain_condition_matches(&self, text: &str, c: &FilterCondition) -> bool {
        match c.operator {
            FilterOp::Contains => match &c.value {
                FilterValue::String(s) => text.contains(s.as_str()),
                _ => false,
            },
            FilterOp::NotContains => match &c.value {
                FilterValue::String(s) => !text.contains(s.as_str()),
                _ => false,
            },
            FilterOp::RegexMatch => c.regex.as_ref().unwrap().is_match(text),
            FilterOp::NotRegexMatch => !c.regex.as_ref().unwrap().is_match(text),
            _ => false,
        }
    }

    fn json_expr_matches(&self, text: &str, expr: &JsonExpr) -> bool {
        match expr {
            JsonExpr::Condition(c) => self.json_value_matches(text, c),
            JsonExpr::And(l, r) => {
                self.json_expr_matches(text, l) && self.json_expr_matches(text, r)
            }
            JsonExpr::Or(l, r) => {
                self.json_expr_matches(text, l) || self.json_expr_matches(text, r)
            }
        }
    }

    fn json_value_matches(&self, text: &str, condition: &FilterCondition) -> bool {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return false;
        };
        let key = condition.json_key.as_deref().unwrap();
        let target = lookup_json_key(&value, key);
        let Some(target) = target else {
            return matches!(
                condition.operator,
                FilterOp::JsonNotEquals | FilterOp::JsonNotRegexMatch
            );
        };
        match condition.operator {
            FilterOp::JsonEquals => compare_json_value(target, &condition.value),
            FilterOp::JsonNotEquals => !compare_json_value(target, &condition.value),
            FilterOp::JsonRegexMatch => {
                let s = json_value_to_string(target);
                condition.regex.as_ref().unwrap().is_match(&s)
            }
            FilterOp::JsonNotRegexMatch => {
                let s = json_value_to_string(target);
                !condition.regex.as_ref().unwrap().is_match(&s)
            }
            _ => false,
        }
    }

    fn filtered_indices(&mut self) -> Vec<usize> {
        if self.filtered_indices_cache.is_none() {
            self.filtered_indices_cache = Some(self.compute_filtered_indices());
        }
        self.filtered_indices_cache.as_ref().unwrap().clone()
    }

    fn compute_filtered_indices(&self) -> Vec<usize> {
        match self.active_filter_query() {
            Some(q) if !q.segments.is_empty() => self
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
                Ok(query) if !query.segments.is_empty() => {
                    self.live_filter_query = Some(query);
                    self.live_filter_error = None;
                    self.filtered_indices_cache = None;
                }
                Ok(_) => {
                    self.live_filter_query = None;
                    self.live_filter_error = None;
                    self.filtered_indices_cache = None;
                }
                Err(msg) => {
                    // Keep the previous live_filter_query so results stay filtered
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
                        Ok(query) if !query.segments.is_empty() => {
                            self.filter_query = Some(query);
                            self.filter_error = None;
                            self.live_filter_query = None;
                            self.live_filter_error = None;
                            self.filtered_indices_cache = None;
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
                            self.filtered_indices_cache = None;
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
                self.filtered_indices_cache = None;
            }
            KeyCode::Backspace => {
                if let Some(input) = &mut self.filter_input {
                    if input.is_empty() {
                        // Stay in filter input mode when input is empty
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
                self.filtered_indices_cache = None;
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
                self.filtered_indices_cache = None;
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
            let cursor_x = (1 + input.len()) as u16;
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
            format!(" {}", input),
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
                " Bksp delete  C-c:cancel  syntax: |= \"text\"  |~ /regex/  != !~  | key = !=  and/or ()"
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

    fn plain(condition: FilterCondition) -> FilterSegment {
        FilterSegment::Plain(condition)
    }

    fn json(condition: FilterCondition) -> FilterSegment {
        FilterSegment::Json(JsonExpr::Condition(condition))
    }

    fn get_plain(query: &FilterQuery, idx: usize) -> &FilterCondition {
        match &query.segments[idx] {
            FilterSegment::Plain(c) => c,
            _ => panic!("Expected Plain segment at index {}", idx),
        }
    }

    fn get_json(query: &FilterQuery, idx: usize) -> &FilterCondition {
        match &query.segments[idx] {
            FilterSegment::Json(JsonExpr::Condition(c)) => c,
            _ => panic!("Expected Json Condition segment at index {}", idx),
        }
    }

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
            segments: vec![plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("alice".to_string()),
                regex: None,
                json_key: None,
            })],
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
            segments: vec![plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("xyz".to_string()),
                regex: None,
                json_key: None,
            })],
        });
        let filtered = app.filtered_indices();
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_clear() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        app.filter_query = Some(FilterQuery {
            segments: vec![plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("xyz".to_string()),
                regex: None,
                json_key: None,
            })],
        });
        assert_eq!(app.filtered_indices().len(), 0);
        app.filter_query = None;
        app.filtered_indices_cache = None;
        assert_eq!(app.filtered_indices().len(), 1);
    }

    #[test]
    fn test_regex_filter_matching() {
        let mut app = App::new(100);
        app.add_line("error: connection timeout".to_string());
        app.add_line("info: request ok".to_string());
        app.add_line("error: disk full".to_string());
        app.filter_query = Some(FilterQuery {
            segments: vec![plain(FilterCondition {
                operator: FilterOp::RegexMatch,
                value: FilterValue::String("err.*timeout".to_string()),
                regex: regex::Regex::new("err.*timeout").ok(),
                json_key: None,
            })],
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
            segments: vec![plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("aaa".to_string()),
                regex: None,
                json_key: None,
            })],
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
            segments: vec![plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("aaa".to_string()),
                regex: None,
                json_key: None,
            })],
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

    #[test]
    fn test_filtered_indices_cache_avoids_recomputation() {
        let mut app = App::new(10000);
        for i in 0..10000 {
            app.add_line(format!(
                r#"{{"level":"{}","msg":"line {}"}}"#,
                if i % 3 == 0 { "error" } else { "info" },
                i
            ));
        }
        app.filter_query = Some(FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonEquals,
                value: FilterValue::String("error".to_string()),
                regex: None,
                json_key: Some("level".to_string()),
            })],
        });

        // First call: computes and caches (may be slow)
        let first = app.filtered_indices();
        assert!(!first.is_empty());

        // Second call: should be served from cache (fast)
        let start = std::time::Instant::now();
        let second = app.filtered_indices();
        let cached_elapsed = start.elapsed();
        assert_eq!(first, second, "cached result should match");
        assert!(
            cached_elapsed.as_micros() < 1000,
            "cached filtered_indices took {:?}, expected < 1ms",
            cached_elapsed
        );
    }

    // Parser tests

    #[test]
    fn test_parse_contains() {
        let query = parse_filter_query(r#"|= "foo""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_plain(&query, 0).operator, FilterOp::Contains);
        assert_eq!(
            get_plain(&query, 0).value,
            FilterValue::String("foo".to_string())
        );
    }

    #[test]
    fn test_parse_regex_match() {
        let query = parse_filter_query(r#"|~ "err.*""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_plain(&query, 0).operator, FilterOp::RegexMatch);
        assert_eq!(
            get_plain(&query, 0).value,
            FilterValue::String("err.*".to_string())
        );
        assert!(get_plain(&query, 0).regex.is_some());
    }

    #[test]
    fn test_parse_not_contains() {
        let query = parse_filter_query(r#"!= "bar""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_plain(&query, 0).operator, FilterOp::NotContains);
        assert_eq!(
            get_plain(&query, 0).value,
            FilterValue::String("bar".to_string())
        );
    }

    #[test]
    fn test_parse_not_regex_match() {
        let query = parse_filter_query(r#"!~ "baz""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_plain(&query, 0).operator, FilterOp::NotRegexMatch);
        assert_eq!(
            get_plain(&query, 0).value,
            FilterValue::String("baz".to_string())
        );
        assert!(get_plain(&query, 0).regex.is_some());
    }

    #[test]
    fn test_parse_multiple_conditions() {
        let query = parse_filter_query(r#"|= "foo" != "bar""#).unwrap();
        assert_eq!(query.segments.len(), 2);
        assert_eq!(get_plain(&query, 0).operator, FilterOp::Contains);
        assert_eq!(
            get_plain(&query, 0).value,
            FilterValue::String("foo".to_string())
        );
        assert_eq!(get_plain(&query, 1).operator, FilterOp::NotContains);
        assert_eq!(
            get_plain(&query, 1).value,
            FilterValue::String("bar".to_string())
        );
    }

    #[test]
    fn test_parse_empty_input() {
        let query = parse_filter_query("").unwrap();
        assert!(query.segments.is_empty());
    }

    #[test]
    fn test_parse_whitespace_only() {
        let query = parse_filter_query("   ").unwrap();
        assert!(query.segments.is_empty());
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
            segments: vec![plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("foo".to_string()),
                regex: None,
                json_key: None,
            })],
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
            segments: vec![plain(FilterCondition {
                operator: FilterOp::NotContains,
                value: FilterValue::String("foo".to_string()),
                regex: None,
                json_key: None,
            })],
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
            segments: vec![plain(FilterCondition {
                operator: FilterOp::RegexMatch,
                value: FilterValue::String("err.*".to_string()),
                regex: regex::Regex::new("err.*").ok(),
                json_key: None,
            })],
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
            segments: vec![plain(FilterCondition {
                operator: FilterOp::NotRegexMatch,
                value: FilterValue::String("err.*".to_string()),
                regex: regex::Regex::new("err.*").ok(),
                json_key: None,
            })],
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
            segments: vec![
                plain(FilterCondition {
                    operator: FilterOp::Contains,
                    value: FilterValue::String("error".to_string()),
                    regex: None,
                    json_key: None,
                }),
                plain(FilterCondition {
                    operator: FilterOp::NotContains,
                    value: FilterValue::String("timeout".to_string()),
                    regex: None,
                    json_key: None,
                }),
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
            segments: vec![
                plain(FilterCondition {
                    operator: FilterOp::Contains,
                    value: FilterValue::String("foo".to_string()),
                    regex: None,
                    json_key: None,
                }),
                plain(FilterCondition {
                    operator: FilterOp::NotContains,
                    value: FilterValue::String("bar".to_string()),
                    regex: None,
                    json_key: None,
                }),
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
                filtered_indices_cache: None,
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

    // JSON key filter parser tests

    #[test]
    fn test_parse_json_key_equals_string() {
        let query = parse_filter_query(r#"| name = "alice""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_json(&query, 0).operator, FilterOp::JsonEquals);
        assert_eq!(
            get_json(&query, 0).value,
            FilterValue::String("alice".to_string())
        );
        assert_eq!(get_json(&query, 0).json_key, Some("name".to_string()));
        assert!(get_json(&query, 0).regex.is_none());
    }

    #[test]
    fn test_parse_json_key_not_equals() {
        let query = parse_filter_query(r#"| name != "bob""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_json(&query, 0).operator, FilterOp::JsonNotEquals);
        assert_eq!(
            get_json(&query, 0).value,
            FilterValue::String("bob".to_string())
        );
        assert_eq!(get_json(&query, 0).json_key, Some("name".to_string()));
    }

    #[test]
    fn test_parse_json_key_regex_match() {
        let query = parse_filter_query(r#"| msg =~ "err.*""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_json(&query, 0).operator, FilterOp::JsonRegexMatch);
        assert_eq!(
            get_json(&query, 0).value,
            FilterValue::String("err.*".to_string())
        );
        assert_eq!(get_json(&query, 0).json_key, Some("msg".to_string()));
        assert!(get_json(&query, 0).regex.is_some());
    }

    #[test]
    fn test_parse_json_key_not_regex_match() {
        let query = parse_filter_query(r#"| msg !~ "err.*""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_json(&query, 0).operator, FilterOp::JsonNotRegexMatch);
        assert_eq!(
            get_json(&query, 0).value,
            FilterValue::String("err.*".to_string())
        );
        assert_eq!(get_json(&query, 0).json_key, Some("msg".to_string()));
        assert!(get_json(&query, 0).regex.is_some());
    }

    #[test]
    fn test_parse_json_key_number() {
        let query = parse_filter_query(r#"| count = 42"#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_json(&query, 0).operator, FilterOp::JsonEquals);
        assert_eq!(get_json(&query, 0).value, FilterValue::Number(42.0));
        assert_eq!(get_json(&query, 0).json_key, Some("count".to_string()));
    }

    #[test]
    fn test_parse_json_key_negative_number() {
        let query = parse_filter_query(r#"| temp = -3"#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_json(&query, 0).value, FilterValue::Number(-3.0));
    }

    #[test]
    fn test_parse_json_key_float() {
        let query = parse_filter_query(r#"| ratio = 2.5"#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_json(&query, 0).value, FilterValue::Number(2.5));
    }

    #[test]
    fn test_parse_json_key_boolean_true() {
        let query = parse_filter_query(r#"| active = true"#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_json(&query, 0).value, FilterValue::Boolean(true));
    }

    #[test]
    fn test_parse_json_key_boolean_false() {
        let query = parse_filter_query(r#"| active = false"#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_json(&query, 0).value, FilterValue::Boolean(false));
    }

    #[test]
    fn test_parse_json_key_null() {
        let query = parse_filter_query(r#"| result = null"#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_json(&query, 0).value, FilterValue::Null);
    }

    #[test]
    fn test_parse_json_nested_key() {
        let query = parse_filter_query(r#"| user.name = "alice""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        assert_eq!(get_json(&query, 0).json_key, Some("user.name".to_string()));
    }

    #[test]
    fn test_parse_mixed_filters() {
        let query = parse_filter_query(r#"|= "foo" | key1 = "value1" != "bar""#).unwrap();
        assert_eq!(query.segments.len(), 3);
        assert_eq!(get_plain(&query, 0).operator, FilterOp::Contains);
        assert_eq!(get_plain(&query, 0).json_key, None);
        assert_eq!(get_json(&query, 1).operator, FilterOp::JsonEquals);
        assert_eq!(get_json(&query, 1).json_key, Some("key1".to_string()));
        assert_eq!(get_plain(&query, 2).operator, FilterOp::NotContains);
        assert_eq!(get_plain(&query, 2).json_key, None);
    }

    #[test]
    fn test_parse_json_key_with_underscore_and_hyphen() {
        let query = parse_filter_query(r#"| my_key-name = "value""#).unwrap();
        assert_eq!(
            get_json(&query, 0).json_key,
            Some("my_key-name".to_string())
        );
    }

    #[test]
    fn test_parse_error_json_key_missing_value() {
        assert!(parse_filter_query(r#"| key ="#).is_err());
    }

    #[test]
    fn test_parse_error_json_key_invalid_regex() {
        assert!(parse_filter_query(r#"| key =~ "[invalid""#).is_err());
    }

    // JSON key filter matching tests

    #[test]
    fn test_json_key_equals_string_match() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonEquals,
                value: FilterValue::String("alice".to_string()),
                regex: None,
                json_key: Some("name".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"name":"alice"}"#));
    }

    #[test]
    fn test_json_key_equals_string_no_match() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonEquals,
                value: FilterValue::String("bob".to_string()),
                regex: None,
                json_key: Some("name".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(!app.line_matches_filter(r#"{"name":"alice"}"#));
    }

    #[test]
    fn test_json_key_not_equals() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonNotEquals,
                value: FilterValue::String("bob".to_string()),
                regex: None,
                json_key: Some("name".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"name":"alice"}"#));
        assert!(!app.line_matches_filter(r#"{"name":"bob"}"#));
    }

    #[test]
    fn test_json_key_number_match() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonEquals,
                value: FilterValue::Number(30.0),
                regex: None,
                json_key: Some("age".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"age":30}"#));
        assert!(!app.line_matches_filter(r#"{"age":"30"}"#));
    }

    #[test]
    fn test_json_key_boolean_match() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonEquals,
                value: FilterValue::Boolean(true),
                regex: None,
                json_key: Some("active".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"active":true}"#));
        assert!(!app.line_matches_filter(r#"{"active":false}"#));
        assert!(!app.line_matches_filter(r#"{"active":"true"}"#));
    }

    #[test]
    fn test_json_key_null_match() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonEquals,
                value: FilterValue::Null,
                regex: None,
                json_key: Some("result".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"result":null}"#));
        assert!(!app.line_matches_filter(r#"{"result":"null"}"#));
    }

    #[test]
    fn test_json_key_nested_match() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonEquals,
                value: FilterValue::String("alice".to_string()),
                regex: None,
                json_key: Some("user.name".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"user":{"name":"alice"}}"#));
        assert!(!app.line_matches_filter(r#"{"user":{"name":"bob"}}"#));
    }

    #[test]
    fn test_json_key_missing_key_equals() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonEquals,
                value: FilterValue::String("alice".to_string()),
                regex: None,
                json_key: Some("missing".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(!app.line_matches_filter(r#"{"name":"alice"}"#));
    }

    #[test]
    fn test_json_key_missing_key_not_equals() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonNotEquals,
                value: FilterValue::String("bob".to_string()),
                regex: None,
                json_key: Some("missing".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        // Key doesn't exist, so "not equals bob" is true
        assert!(app.line_matches_filter(r#"{"name":"alice"}"#));
    }

    #[test]
    fn test_json_key_non_json_line() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonEquals,
                value: FilterValue::String("alice".to_string()),
                regex: None,
                json_key: Some("name".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(!app.line_matches_filter("plain text line"));
    }

    #[test]
    fn test_json_key_regex_match() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonRegexMatch,
                value: FilterValue::String("err.*".to_string()),
                regex: regex::Regex::new("err.*").ok(),
                json_key: Some("msg".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"msg":"error: timeout"}"#));
        assert!(!app.line_matches_filter(r#"{"msg":"info: ok"}"#));
    }

    #[test]
    fn test_json_key_regex_on_number() {
        let query = FilterQuery {
            segments: vec![json(FilterCondition {
                operator: FilterOp::JsonRegexMatch,
                value: FilterValue::String("4.*".to_string()),
                regex: regex::Regex::new("4.*").ok(),
                json_key: Some("count".to_string()),
            })],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"count":42}"#));
        assert!(!app.line_matches_filter(r#"{"count":13}"#));
    }

    #[test]
    fn test_mixed_filter_and_json_key() {
        let query = FilterQuery {
            segments: vec![
                plain(FilterCondition {
                    operator: FilterOp::Contains,
                    value: FilterValue::String("error".to_string()),
                    regex: None,
                    json_key: None,
                }),
                json(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::String("timeout".to_string()),
                    regex: None,
                    json_key: Some("type".to_string()),
                }),
            ],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"type":"timeout","msg":"error occurred"}"#));
        assert!(!app.line_matches_filter(r#"{"type":"disk","msg":"error occurred"}"#));
        assert!(!app.line_matches_filter(r#"{"type":"timeout","msg":"info ok"}"#));
    }

    #[test]
    fn test_json_key_display_string() {
        let query = FilterQuery {
            segments: vec![
                json(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::String("alice".to_string()),
                    regex: None,
                    json_key: Some("name".to_string()),
                }),
                json(FilterCondition {
                    operator: FilterOp::JsonNotEquals,
                    value: FilterValue::Number(42.0),
                    regex: None,
                    json_key: Some("age".to_string()),
                }),
            ],
        };
        assert_eq!(query.display_string(), r#"| name = "alice" | age != 42"#);
    }

    #[test]
    fn test_json_key_display_boolean_and_null() {
        let query = FilterQuery {
            segments: vec![
                json(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::Boolean(true),
                    regex: None,
                    json_key: Some("active".to_string()),
                }),
                json(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::Null,
                    regex: None,
                    json_key: Some("result".to_string()),
                }),
            ],
        };
        assert_eq!(query.display_string(), "| active = true | result = null");
    }

    #[test]
    fn test_live_filter_keeps_previous_valid_on_parse_error() {
        let mut app = App::new(100);
        app.add_line("foo line".to_string());
        app.add_line("bar line".to_string());
        app.add_line("foo bar line".to_string());

        // Start filter input mode with a valid query
        app.filter_input = Some(r#"|= "foo""#.to_string());
        app.update_live_filter();
        assert!(app.live_filter_query.is_some());
        assert!(app.live_filter_error.is_none());
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0, 2]);

        // Add invalid suffix: parse error, but previous valid query should be kept
        app.filter_input = Some(r#"|= "foo" |"#.to_string());
        app.update_live_filter();
        assert!(
            app.live_filter_query.is_some(),
            "live_filter_query should keep previous valid query on parse error"
        );
        assert!(app.live_filter_error.is_some());
        let filtered = app.filtered_indices();
        assert_eq!(
            filtered,
            vec![0, 2],
            "should still filter by previous valid query"
        );

        // Continue typing to make it valid again with two conditions
        app.filter_input = Some(r#"|= "foo" |= "bar""#.to_string());
        app.update_live_filter();
        assert!(app.live_filter_query.is_some());
        assert!(app.live_filter_error.is_none());
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![2]);
    }

    #[test]
    fn test_live_filter_no_previous_keeps_none_on_error() {
        let mut app = App::new(100);
        // No previous valid query: error with live_filter_query still None
        app.filter_input = Some("|".to_string());
        app.update_live_filter();
        assert!(app.live_filter_query.is_none());
        assert!(app.live_filter_error.is_some());
    }

    // and/or/parens parser tests

    #[test]
    fn test_parse_json_and() {
        let query = parse_filter_query(r#"| key1 = "foo" and key2 = "bar""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        match &query.segments[0] {
            FilterSegment::Json(JsonExpr::And(l, r)) => match (l.as_ref(), r.as_ref()) {
                (JsonExpr::Condition(a), JsonExpr::Condition(b)) => {
                    assert_eq!(a.json_key, Some("key1".to_string()));
                    assert_eq!(b.json_key, Some("key2".to_string()));
                }
                _ => panic!("Expected Condition children"),
            },
            _ => panic!("Expected Json And segment"),
        }
    }

    #[test]
    fn test_parse_json_or() {
        let query = parse_filter_query(r#"| key1 = "foo" or key2 = "bar""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        match &query.segments[0] {
            FilterSegment::Json(JsonExpr::Or(l, r)) => match (l.as_ref(), r.as_ref()) {
                (JsonExpr::Condition(a), JsonExpr::Condition(b)) => {
                    assert_eq!(a.json_key, Some("key1".to_string()));
                    assert_eq!(b.json_key, Some("key2".to_string()));
                }
                _ => panic!("Expected Condition children"),
            },
            _ => panic!("Expected Json Or segment"),
        }
    }

    #[test]
    fn test_parse_json_parens_or_and() {
        let query =
            parse_filter_query(r#"| (key1 = "foo" or key2 = "bar") and key3 = "baz""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        match &query.segments[0] {
            FilterSegment::Json(JsonExpr::And(l, r)) => match (l.as_ref(), r.as_ref()) {
                (JsonExpr::Or(ll, lr), JsonExpr::Condition(c)) => {
                    match (ll.as_ref(), lr.as_ref()) {
                        (JsonExpr::Condition(a), JsonExpr::Condition(b)) => {
                            assert_eq!(a.json_key, Some("key1".to_string()));
                            assert_eq!(b.json_key, Some("key2".to_string()));
                        }
                        _ => panic!("Expected Condition children in Or"),
                    }
                    assert_eq!(c.json_key, Some("key3".to_string()));
                }
                _ => panic!("Expected Or and Condition children"),
            },
            _ => panic!("Expected Json And segment"),
        }
    }

    #[test]
    fn test_parse_plain_json_and_plain() {
        let query = parse_filter_query(r#"|= "foo" | k1 = "bar" and k2 = 10 |= "bar""#).unwrap();
        assert_eq!(query.segments.len(), 3);
        assert_eq!(get_plain(&query, 0).operator, FilterOp::Contains);
        assert_eq!(get_plain(&query, 2).operator, FilterOp::Contains);
        match &query.segments[1] {
            FilterSegment::Json(JsonExpr::And(l, r)) => match (l.as_ref(), r.as_ref()) {
                (JsonExpr::Condition(a), JsonExpr::Condition(b)) => {
                    assert_eq!(a.json_key, Some("k1".to_string()));
                    assert_eq!(b.json_key, Some("k2".to_string()));
                    assert_eq!(b.value, FilterValue::Number(10.0));
                }
                _ => panic!("Expected Condition children"),
            },
            _ => panic!("Expected Json And segment at index 1"),
        }
    }

    #[test]
    fn test_parse_plain_json_or_plain() {
        let query = parse_filter_query(r#"|= "foo" | k1 = "bar" or k2 = 10 != "bar""#).unwrap();
        assert_eq!(query.segments.len(), 3);
        assert_eq!(get_plain(&query, 0).operator, FilterOp::Contains);
        assert_eq!(get_plain(&query, 2).operator, FilterOp::NotContains);
        match &query.segments[1] {
            FilterSegment::Json(JsonExpr::Or(_, _)) => {}
            _ => panic!("Expected Json Or segment at index 1"),
        }
    }

    #[test]
    fn test_parse_json_grouped_or_and_plain_json() {
        let query =
            parse_filter_query(r#"|= "foo" | (k1 = "bar" or k2 = 10) != "bar" | k3 = true"#)
                .unwrap();
        assert_eq!(query.segments.len(), 4);
        assert_eq!(get_plain(&query, 0).operator, FilterOp::Contains);
        assert_eq!(get_plain(&query, 2).operator, FilterOp::NotContains);
        match &query.segments[1] {
            FilterSegment::Json(JsonExpr::Or(_, _)) => {}
            _ => panic!("Expected Json Or segment at index 1"),
        }
        match &query.segments[3] {
            FilterSegment::Json(JsonExpr::Condition(c)) => {
                assert_eq!(c.json_key, Some("k3".to_string()));
                assert_eq!(c.value, FilterValue::Boolean(true));
            }
            _ => panic!("Expected Json Condition at index 3"),
        }
    }

    #[test]
    fn test_parse_error_plain_and_plain() {
        assert!(parse_filter_query(r#"|= "foo" and |= "bar""#).is_err());
    }

    #[test]
    fn test_parse_error_plain_and_not() {
        assert!(parse_filter_query(r#"|= "foo" and != "bar""#).is_err());
    }

    #[test]
    fn test_parse_error_plain_or_not() {
        assert!(parse_filter_query(r#"|= "foo" or != "bar""#).is_err());
    }

    #[test]
    fn test_parse_error_regex_or_not_regex() {
        assert!(parse_filter_query(r#"|~ "foo" or !~ "bar""#).is_err());
    }

    #[test]
    fn test_parse_error_json_and_plain() {
        assert!(parse_filter_query(r#"|= "foo" | k1 = "bar" and |= "bar""#).is_err());
    }

    #[test]
    fn test_parse_error_unmatched_paren() {
        assert!(parse_filter_query(r#"| (k1 = "foo""#).is_err());
    }

    // and/or evaluation tests

    #[test]
    fn test_json_and_matches_both() {
        let query = FilterQuery {
            segments: vec![FilterSegment::Json(JsonExpr::And(
                Box::new(JsonExpr::Condition(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::String("foo".to_string()),
                    regex: None,
                    json_key: Some("k1".to_string()),
                })),
                Box::new(JsonExpr::Condition(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::String("bar".to_string()),
                    regex: None,
                    json_key: Some("k2".to_string()),
                })),
            ))],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"k1":"foo","k2":"bar"}"#));
        assert!(!app.line_matches_filter(r#"{"k1":"foo","k2":"baz"}"#));
        assert!(!app.line_matches_filter(r#"{"k1":"other","k2":"bar"}"#));
    }

    #[test]
    fn test_json_or_matches_either() {
        let query = FilterQuery {
            segments: vec![FilterSegment::Json(JsonExpr::Or(
                Box::new(JsonExpr::Condition(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::String("foo".to_string()),
                    regex: None,
                    json_key: Some("k1".to_string()),
                })),
                Box::new(JsonExpr::Condition(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::String("bar".to_string()),
                    regex: None,
                    json_key: Some("k2".to_string()),
                })),
            ))],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"k1":"foo","k2":"baz"}"#));
        assert!(app.line_matches_filter(r#"{"k1":"other","k2":"bar"}"#));
        assert!(!app.line_matches_filter(r#"{"k1":"other","k2":"baz"}"#));
    }

    #[test]
    fn test_json_or_in_parens_with_and() {
        let query = FilterQuery {
            segments: vec![FilterSegment::Json(JsonExpr::And(
                Box::new(JsonExpr::Or(
                    Box::new(JsonExpr::Condition(FilterCondition {
                        operator: FilterOp::JsonEquals,
                        value: FilterValue::String("timeout".to_string()),
                        regex: None,
                        json_key: Some("type".to_string()),
                    })),
                    Box::new(JsonExpr::Condition(FilterCondition {
                        operator: FilterOp::JsonEquals,
                        value: FilterValue::String("disk".to_string()),
                        regex: None,
                        json_key: Some("type".to_string()),
                    })),
                )),
                Box::new(JsonExpr::Condition(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::Boolean(true),
                    regex: None,
                    json_key: Some("critical".to_string()),
                })),
            ))],
        };
        let app = App {
            filter_query: Some(query),
            ..App::new(100)
        };
        assert!(app.line_matches_filter(r#"{"type":"timeout","critical":true}"#));
        assert!(app.line_matches_filter(r#"{"type":"disk","critical":true}"#));
        assert!(!app.line_matches_filter(r#"{"type":"timeout","critical":false}"#));
        assert!(!app.line_matches_filter(r#"{"type":"network","critical":true}"#));
    }

    // and/or display tests

    #[test]
    fn test_json_and_display_string() {
        let query = FilterQuery {
            segments: vec![FilterSegment::Json(JsonExpr::And(
                Box::new(JsonExpr::Condition(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::String("foo".to_string()),
                    regex: None,
                    json_key: Some("k1".to_string()),
                })),
                Box::new(JsonExpr::Condition(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::Number(42.0),
                    regex: None,
                    json_key: Some("k2".to_string()),
                })),
            ))],
        };
        assert_eq!(query.display_string(), r#"| k1 = "foo" and k2 = 42"#);
    }

    #[test]
    fn test_json_or_display_string() {
        let query = FilterQuery {
            segments: vec![FilterSegment::Json(JsonExpr::Or(
                Box::new(JsonExpr::Condition(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::String("foo".to_string()),
                    regex: None,
                    json_key: Some("k1".to_string()),
                })),
                Box::new(JsonExpr::Condition(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::String("bar".to_string()),
                    regex: None,
                    json_key: Some("k2".to_string()),
                })),
            ))],
        };
        assert_eq!(query.display_string(), r#"| k1 = "foo" or k2 = "bar""#);
    }

    #[test]
    fn test_json_or_in_and_display_string() {
        let query = FilterQuery {
            segments: vec![FilterSegment::Json(JsonExpr::And(
                Box::new(JsonExpr::Or(
                    Box::new(JsonExpr::Condition(FilterCondition {
                        operator: FilterOp::JsonEquals,
                        value: FilterValue::String("foo".to_string()),
                        regex: None,
                        json_key: Some("k1".to_string()),
                    })),
                    Box::new(JsonExpr::Condition(FilterCondition {
                        operator: FilterOp::JsonEquals,
                        value: FilterValue::String("bar".to_string()),
                        regex: None,
                        json_key: Some("k2".to_string()),
                    })),
                )),
                Box::new(JsonExpr::Condition(FilterCondition {
                    operator: FilterOp::JsonEquals,
                    value: FilterValue::Boolean(true),
                    regex: None,
                    json_key: Some("k3".to_string()),
                })),
            ))],
        };
        assert_eq!(
            query.display_string(),
            r#"| (k1 = "foo" or k2 = "bar") and k3 = true"#
        );
    }
}
