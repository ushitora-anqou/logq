use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;

use crate::filter::*;
use crate::input::LineSource;

const STDERR_PREFIX: &str = "[stderr] ";
const SYSTEM_PREFIX: &str = "[logq] ";

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub text: String,
    pub timestamp: String, // "HH:MM:SS.mmm"
    pub source: LineSource,
}

impl LogEntry {
    pub fn source_prefix(&self) -> (&str, usize, Style) {
        match self.source {
            LineSource::Stdout => ("", 0, Style::default()),
            LineSource::Stderr => (
                STDERR_PREFIX,
                STDERR_PREFIX.len(),
                Style::default().fg(ratatui::style::Color::Red),
            ),
            LineSource::System => (
                SYSTEM_PREFIX,
                SYSTEM_PREFIX.len(),
                Style::default().fg(ratatui::style::Color::Yellow),
            ),
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

pub struct FilterState {
    pub filter_input: Option<tui_input::Input>,
    pub filter_query: Option<FilterQuery>,
    pub filter_error: Option<String>,
    pub live_filter_query: Option<FilterQuery>,
    pub live_filter_error: Option<String>,
    pub filter_history: Vec<String>,
    pub filter_history_index: Option<usize>,
    pub filter_draft: Option<tui_input::Input>,
    pub history_search_pattern: Option<String>,
    pub history_search_original_input: Option<tui_input::Input>,
    pub history_search_failed: bool,
    pub history_search_start: Option<usize>,
    pub filter_raw_input: Option<String>,
}

impl Default for FilterState {
    fn default() -> Self {
        Self::new()
    }
}

impl FilterState {
    pub fn new() -> Self {
        Self {
            filter_input: None,
            filter_query: None,
            filter_error: None,
            live_filter_query: None,
            live_filter_error: None,
            filter_history: Vec::new(),
            filter_history_index: None,
            filter_draft: None,
            history_search_pattern: None,
            history_search_original_input: None,
            history_search_failed: false,
            history_search_start: None,
            filter_raw_input: None,
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

    pub fn active_filter_query(&self) -> Option<&FilterQuery> {
        if self.filter_input.is_some() {
            self.live_filter_query.as_ref()
        } else {
            self.filter_query.as_ref()
        }
    }

    pub fn line_matches_filter(&self, text: &str) -> bool {
        self.active_filter_query().is_none_or(|q| q.matches(text))
    }

    pub fn active_line_format(&self) -> Option<&LineFormatTemplate> {
        self.active_filter_query().and_then(|q| {
            q.segments.iter().find_map(|seg| match seg {
                FilterSegment::LineFormat(t) => Some(t),
                _ => None,
            })
        })
    }

    pub fn update_live_filter(&mut self) -> bool {
        let mut invalidated = false;
        if let Some(input) = &self.filter_input {
            match parse_filter_query(input.value()) {
                Ok(query) if !query.segments.is_empty() => {
                    self.live_filter_query = Some(query);
                    self.live_filter_error = None;
                    invalidated = true;
                }
                Ok(_) => {
                    self.live_filter_query = None;
                    self.live_filter_error = None;
                    invalidated = true;
                }
                Err(msg) => {
                    // Keep the previous live_filter_query so results stay filtered
                    self.live_filter_error = Some(msg);
                }
            }
        }
        invalidated
    }

    pub fn start_filter_input(&mut self) {
        let initial = match &self.filter_raw_input {
            Some(raw) => raw.clone(),
            None => self
                .filter_query
                .as_ref()
                .map(|q| q.display_string())
                .unwrap_or_default(),
        };
        self.filter_input = Some(tui_input::Input::new(initial));
        self.filter_history_index = None;
        self.filter_draft = None;
        self.clear_history_search();
    }

    /// Returns true if filter was successfully applied (committed).
    /// Returns false if there was a parse error (input is preserved).
    pub fn apply_filter_submit(&mut self) -> bool {
        let Some(input) = self.filter_input.take() else {
            self.reset_filter_input_state();
            return true;
        };
        let value = input.value().to_string();
        match parse_filter_query(&value) {
            Ok(query) if !query.segments.is_empty() => {
                self.filter_query = Some(query);
                self.filter_error = None;
                self.live_filter_query = None;
                self.live_filter_error = None;
                self.filter_raw_input = Some(value.clone());
                if self.filter_history.last() != Some(&value) {
                    self.filter_history.push(value);
                    if self.filter_history.len() > 100 {
                        self.filter_history.remove(0);
                    }
                }
                self.reset_filter_input_state();
                true
            }
            Ok(_) => {
                self.filter_query = None;
                self.filter_error = None;
                self.live_filter_query = None;
                self.live_filter_error = None;
                self.filter_raw_input = None;
                self.reset_filter_input_state();
                true
            }
            Err(msg) => {
                self.filter_error = Some(msg.clone());
                self.live_filter_error = Some(msg);
                self.filter_input = Some(input);
                false
            }
        }
    }

    pub fn cancel_filter_input(&mut self) {
        self.filter_input = None;
        self.filter_error = None;
        self.live_filter_query = None;
        self.live_filter_error = None;
        self.reset_filter_input_state();
    }

    pub fn handle_history_up(&mut self) {
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
            self.filter_input = Some(tui_input::Input::new(
                self.filter_history[current - 1].clone(),
            ));
            self.update_live_filter();
        }
    }

    pub fn handle_history_down(&mut self) {
        if self.filter_history.is_empty() {
            return;
        }
        let current = self
            .filter_history_index
            .unwrap_or(self.filter_history.len());
        if current < self.filter_history.len() - 1 {
            self.filter_history_index = Some(current + 1);
            self.filter_input = Some(tui_input::Input::new(
                self.filter_history[current + 1].clone(),
            ));
            self.update_live_filter();
        } else {
            // Past the end: restore draft
            self.filter_history_index = None;
            self.filter_input = self
                .filter_draft
                .clone()
                .or_else(|| Some(tui_input::Input::default()));
            self.update_live_filter();
        }
    }

    pub fn handle_history_search(&mut self) {
        if self.filter_history.is_empty() {
            return;
        }

        // Activate search mode if not already active
        if self.history_search_pattern.is_none() {
            self.history_search_pattern = self.filter_input.as_ref().map(|i| i.value().to_string());
            self.history_search_original_input = self.filter_input.clone();
            self.history_search_start = None;
        }

        let pattern = self.history_search_pattern.as_deref().unwrap_or("");
        let start = self
            .history_search_start
            .unwrap_or(self.filter_history.len());

        // Search backwards from current position
        for i in (0..start).rev() {
            if self.filter_history[i].contains(pattern) {
                self.filter_input = Some(tui_input::Input::new(self.filter_history[i].clone()));
                self.history_search_start = Some(i);
                self.history_search_failed = false;
                self.update_live_filter();
                return;
            }
        }
        // Wrap around: try from the end
        if start < self.filter_history.len() {
            for i in (start..self.filter_history.len()).rev() {
                if self.filter_history[i].contains(pattern) {
                    self.filter_input = Some(tui_input::Input::new(self.filter_history[i].clone()));
                    self.history_search_start = Some(i);
                    self.history_search_failed = false;
                    self.update_live_filter();
                    return;
                }
            }
        }
        // No match found
        self.history_search_failed = true;
    }

    pub fn history_search_update(&mut self) {
        // Re-search from the end of history with the updated pattern
        let pattern = self.history_search_pattern.as_deref().unwrap_or("");
        for i in (0..self.filter_history.len()).rev() {
            if self.filter_history[i].contains(pattern) {
                self.filter_input = Some(tui_input::Input::new(self.filter_history[i].clone()));
                self.history_search_start = Some(i);
                self.history_search_failed = false;
                self.update_live_filter();
                return;
            }
        }
        // No match
        self.history_search_failed = true;
        self.filter_input = self.history_search_original_input.clone();
        self.update_live_filter();
    }

    pub fn clear_history_search(&mut self) {
        self.history_search_pattern = None;
        self.history_search_original_input = None;
        self.history_search_failed = false;
        self.history_search_start = None;
    }

    fn reset_filter_input_state(&mut self) {
        self.filter_draft = None;
        self.filter_history_index = None;
        self.clear_history_search();
    }

    /// Handle a key event during filter input mode.
    /// Returns a tuple of (wants_invalidate_caches, toggle_help, quit).
    pub fn handle_filter_key(&mut self, key: KeyEvent) -> (bool, bool, bool) {
        // History search mode: characters go to search pattern
        if let Some(pattern) = &mut self.history_search_pattern {
            match key.code {
                KeyCode::Enter => {
                    self.apply_filter_submit();
                    return (true, false, false);
                }
                KeyCode::Esc => {
                    // Accept the current match, exit search only
                    self.history_search_pattern = None;
                    self.history_search_original_input = None;
                    self.history_search_failed = false;
                    self.history_search_start = None;
                    return (false, false, false);
                }
                KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.handle_history_search();
                    return (false, false, false);
                }
                KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.filter_input = self.history_search_original_input.take();
                    self.history_search_pattern = None;
                    self.history_search_start = None;
                    self.history_search_failed = false;
                    let invalidated = self.update_live_filter();
                    return (invalidated, false, false);
                }
                KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return (false, false, true);
                }
                KeyCode::Backspace => {
                    pattern.pop();
                    self.history_search_update();
                    return (false, false, false);
                }
                KeyCode::Char(c) => {
                    pattern.push(c);
                    self.history_search_update();
                    return (false, false, false);
                }
                _ => return (false, false, false),
            }
        }

        // Normal editing mode
        match key.code {
            KeyCode::Enter => {
                self.apply_filter_submit();
                (true, false, false)
            }
            KeyCode::Esc => {
                self.cancel_filter_input();
                (true, false, false)
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cancel_filter_input();
                (true, false, false)
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.handle_history_search();
                (false, false, false)
            }
            KeyCode::Up => {
                self.clear_history_search();
                self.handle_history_up();
                (false, false, false)
            }
            KeyCode::Down => {
                self.clear_history_search();
                self.handle_history_down();
                (false, false, false)
            }
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                (false, true, false)
            }
            KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                (false, false, true)
            }
            _ => {
                // Delegate all other editing to tui-input
                let mut invalidated = false;
                if let Some(input) = &mut self.filter_input {
                    use tui_input::backend::crossterm::EventHandler;
                    if input.handle_event(&Event::Key(key)).is_some() {
                        self.filter_error = None;
                        invalidated = self.update_live_filter();
                    }
                }
                (invalidated, false, false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_filter_preserves_raw_input() {
        let mut state = FilterState::new();
        state.filter_input = Some(tui_input::Input::new("MSG".to_string()));
        assert!(state.apply_filter_submit());

        assert!(state.filter_query.is_some());
        assert_eq!(state.filter_raw_input.as_deref(), Some("MSG"));

        state.start_filter_input();
        assert_eq!(state.filter_input.as_ref().unwrap().value(), "MSG");
    }

    #[test]
    fn explicit_operator_filter_preserves_raw_input() {
        let mut state = FilterState::new();
        state.filter_input = Some(tui_input::Input::new("|= \"MSG\"".to_string()));
        assert!(state.apply_filter_submit());

        assert!(state.filter_query.is_some());
        assert_eq!(state.filter_raw_input.as_deref(), Some("|= \"MSG\""));

        state.start_filter_input();
        assert_eq!(state.filter_input.as_ref().unwrap().value(), "|= \"MSG\"");
    }

    #[test]
    fn empty_filter_clears_raw_input() {
        let mut state = FilterState::new();
        state.filter_input = Some(tui_input::Input::new("MSG".to_string()));
        assert!(state.apply_filter_submit());
        assert!(state.filter_raw_input.is_some());

        state.filter_input = Some(tui_input::Input::new(String::new()));
        assert!(state.apply_filter_submit());
        assert!(state.filter_raw_input.is_none());
    }

    #[test]
    fn cancel_preserves_existing_raw_input() {
        let mut state = FilterState::new();
        state.filter_input = Some(tui_input::Input::new("MSG".to_string()));
        assert!(state.apply_filter_submit());
        assert_eq!(state.filter_raw_input.as_deref(), Some("MSG"));

        state.start_filter_input();
        state.cancel_filter_input();
        assert_eq!(state.filter_raw_input.as_deref(), Some("MSG"));

        state.start_filter_input();
        assert_eq!(state.filter_input.as_ref().unwrap().value(), "MSG");
    }
}
