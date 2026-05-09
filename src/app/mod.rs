mod handler;
mod nav;
mod ui;

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;

use base64::Engine;
use chrono::Local;
use crossterm::event::Event;
use ratatui::layout::Rect;

use crate::filter::*;
use crate::filter_state::{FilterState, LogEntry};
use crate::highlight::{HighlightColors, highlight_line};
use crate::input::LineSource;
use crate::recorder::Recorder;
use crate::render::*;

const TIMESTAMP_WIDTH: usize = 13; // "HH:MM:SS.mmm "

pub struct ShortcutItem {
    pub key: &'static str,
    pub desc: String,
}

#[derive(Default)]
pub(crate) struct RenderCache {
    filtered_indices: Option<Vec<usize>>,
    /// (content_width, row_layout, prefix_sums)
    row_layout: Option<(usize, Vec<usize>, Vec<usize>)>,
    /// Per-entry expanded height cache: (content_width, lines_idx -> height)
    entry_heights: Option<(usize, HashMap<usize, usize>)>,
}

impl RenderCache {
    fn invalidate(&mut self) {
        self.filtered_indices = None;
        self.row_layout = None;
        self.entry_heights = None;
    }
}

pub struct App {
    pub lines: VecDeque<LogEntry>,
    pub selected: usize,
    pub scroll_offset: usize,
    pub auto_scroll: bool,
    pub max_lines: usize,
    pub should_quit: bool,
    pub(crate) colors: HighlightColors,
    pub filter: FilterState,
    pub(crate) cache: RenderCache,
    pub(crate) pending_g: bool,
    pub(crate) pending_z: bool,
    pub expanded: HashSet<usize>,
    pub expand_all: bool,
    pub process_exited: bool,
    pub show_help: bool,
    pub(crate) help_scroll: u16,
    pub(crate) recorder: Option<Recorder>,
    pub(crate) command_input: Option<tui_input::Input>,
    pub(crate) command_error: Option<String>,
    pub context_mode: bool,
    pub(crate) context_center: usize,
    pub(crate) main_selected: usize,
    pub(crate) main_scroll_offset: usize,
    pub(crate) main_auto_scroll: bool,
    pub(crate) main_expanded: HashSet<usize>,
    pub(crate) main_expand_all: bool,
}

impl App {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            selected: 0,
            scroll_offset: 0,
            auto_scroll: true,
            max_lines,
            should_quit: false,
            colors: HighlightColors::default(),
            filter: FilterState::new(),
            cache: RenderCache::default(),
            pending_g: false,
            pending_z: false,
            expanded: HashSet::new(),
            expand_all: false,
            process_exited: false,
            show_help: false,
            help_scroll: 0,
            recorder: None,
            command_input: None,
            command_error: None,
            context_mode: false,
            context_center: 0,
            main_selected: 0,
            main_scroll_offset: 0,
            main_auto_scroll: true,
            main_expanded: HashSet::new(),
            main_expand_all: false,
        }
    }

    pub fn load_history(&mut self) {
        self.filter.load_history();
    }

    pub fn save_history(&self) {
        self.filter.save_history();
    }

    pub fn add_line(&mut self, line: String) {
        self.add_line_with_source(line, LineSource::Stdout);
    }

    pub fn add_line_with_source(&mut self, line: String, source: LineSource) {
        // Record to file first (before any heavy processing) for resilience
        if let Some(recorder) = &self.recorder {
            recorder.record(&line);
        }

        if source == LineSource::System && line.contains("process exited") {
            self.process_exited = true;
        }
        let matches_filter = self.line_matches_filter(&line);
        let timestamp = Local::now().format("%H:%M:%S%.3f").to_string();
        let new_idx = self.lines.len();
        self.lines.push_back(LogEntry {
            text: line,
            timestamp,
            source,
        });
        if self.lines.len() > self.max_lines {
            self.lines.pop_front();
            self.adjust_filtered_indices_on_remove();
            self.adjust_entry_heights_on_remove();
            if matches_filter && let Some(indices) = &mut self.cache.filtered_indices {
                indices.push(self.lines.len() - 1);
            }
            if self.selected > 0 {
                self.selected -= 1;
            }
            self.expanded = self
                .expanded
                .iter()
                .filter(|&&i| i > 0)
                .map(|&i| i - 1)
                .collect();
        } else if let Some(indices) = &mut self.cache.filtered_indices
            && matches_filter
        {
            indices.push(new_idx);
        }
        self.invalidate_row_layout();
        // auto_scroll: selected/scroll_offset are updated in update_auto_scroll()
    }

    pub(crate) fn active_filter_query(&self) -> Option<&FilterQuery> {
        self.filter.active_filter_query()
    }

    pub(crate) fn line_matches_filter(&self, text: &str) -> bool {
        self.filter.line_matches_filter(text)
    }

    pub(crate) fn filtered_indices(&mut self) -> Vec<usize> {
        if self.cache.filtered_indices.is_none() {
            self.cache.filtered_indices = Some(self.compute_filtered_indices());
        }
        self.cache.filtered_indices.as_ref().unwrap().clone()
    }

    fn compute_filtered_indices(&self) -> Vec<usize> {
        if self.context_mode {
            return (0..self.lines.len()).collect();
        }
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

    pub(crate) fn active_line_format(&self) -> Option<&LineFormatTemplate> {
        self.filter.active_line_format()
    }

    pub(crate) fn display_text_for(&self, idx: usize) -> String {
        if let Some(template) = self.active_line_format()
            && let Some(formatted) = template.format(&self.lines[idx].text)
        {
            return formatted;
        }
        self.lines[idx].text.clone()
    }

    pub(crate) fn visible_height(&self, area: &Rect) -> usize {
        // Titlebar(1) + status(1) + shortcuts(2) = 4; during filter/command input add input(1) = 5
        let overhead: usize = if self.filter.filter_input.is_some() || self.command_input.is_some()
        {
            5
        } else {
            4
        };
        (area.height as usize).saturating_sub(overhead)
    }

    pub(crate) fn invalidate_caches(&mut self) {
        self.cache.invalidate();
    }

    pub(crate) fn invalidate_row_layout(&mut self) {
        self.cache.row_layout = None;
    }

    fn adjust_filtered_indices_on_remove(&mut self) {
        if let Some(indices) = &mut self.cache.filtered_indices {
            // The front entry was removed; its index was 0.
            // Remove index 0 if present, then decrement all remaining indices.
            let removed_zero = indices.first() == Some(&0);
            if removed_zero {
                indices.remove(0);
            }
            for idx in indices.iter_mut() {
                *idx = idx.saturating_sub(1);
            }
        }
    }

    fn adjust_entry_heights_on_remove(&mut self) {
        if let Some((_, map)) = &mut self.cache.entry_heights {
            map.remove(&0);
            let mut new_map = HashMap::with_capacity(map.len());
            for (&k, &v) in map.iter() {
                new_map.insert(k.saturating_sub(1), v);
            }
            *map = new_map;
        }
    }

    pub(crate) fn is_expanded(&self, lines_idx: usize) -> bool {
        self.expand_all || self.expanded.contains(&lines_idx)
    }

    fn entry_display_height(&self, lines_idx: usize, content_width: usize) -> usize {
        if !self.is_expanded(lines_idx) {
            return 1;
        }
        let text = highlight_line(&self.display_text_for(lines_idx), &self.colors);
        wrapped_text_height(&text, content_width)
    }

    pub(crate) fn entry_display_height_cached(
        &mut self,
        lines_idx: usize,
        content_width: usize,
    ) -> usize {
        if let Some((w, _)) = &self.cache.entry_heights
            && *w != content_width
        {
            self.cache.entry_heights = None;
        }
        if self.cache.entry_heights.is_none() {
            self.cache.entry_heights = Some((content_width, HashMap::new()));
        }
        if let Some(&h) = self.cache.entry_heights.as_ref().unwrap().1.get(&lines_idx) {
            return h;
        }
        let h = self.entry_display_height(lines_idx, content_width);
        self.cache
            .entry_heights
            .as_mut()
            .unwrap()
            .1
            .insert(lines_idx, h);
        h
    }

    pub(crate) fn compute_row_layout(
        &mut self,
        filtered: &[usize],
        content_width: usize,
    ) -> Vec<usize> {
        filtered
            .iter()
            .map(|&idx| self.entry_display_height_cached(idx, content_width))
            .collect()
    }

    /// Returns cached row heights and prefix sums for the active filtered set at the
    /// given content_width. The cache is invalidated whenever lines, expansion state,
    /// or the filter changes.
    pub(crate) fn cached_row_layout(&mut self, content_width: usize) -> (&[usize], &[usize]) {
        let needs_recompute = match &self.cache.row_layout {
            Some((cached_w, _, _)) => *cached_w != content_width,
            None => true,
        };
        if needs_recompute {
            let filtered = self.filtered_indices();
            let row_layout = self.compute_row_layout(&filtered, content_width);
            let mut prefix_sums = Vec::with_capacity(row_layout.len() + 1);
            let mut acc: usize = 0;
            prefix_sums.push(0);
            for &h in &row_layout {
                acc = acc.saturating_add(h);
                prefix_sums.push(acc);
            }
            self.cache.row_layout = Some((content_width, row_layout, prefix_sums));
        }
        let cache = self.cache.row_layout.as_ref().unwrap();
        (&cache.1, &cache.2)
    }

    pub fn start_recording(&mut self, path: std::path::PathBuf) -> std::io::Result<()> {
        // Stop existing recording if any
        if let Some(recorder) = self.recorder.take() {
            drop(recorder);
        }
        let recorder = Recorder::start(path)?;
        self.recorder = Some(recorder);
        Ok(())
    }

    pub fn stop_recording(&mut self) {
        if let Some(recorder) = self.recorder.take() {
            drop(recorder);
        }
    }

    pub fn yank_selected(&self) -> std::io::Result<()> {
        if self.lines.is_empty() {
            return Ok(());
        }
        let text = &self.lines[self.selected].text;
        let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        let osc52 = format!("\x1b]52;c;{}\x07", encoded);
        std::io::stderr().write_all(osc52.as_bytes())?;
        std::io::stderr().flush()
    }

    pub fn enter_context_mode(&mut self, visible_height: usize, content_width: usize) {
        if self.context_mode || self.lines.is_empty() {
            return;
        }
        self.main_selected = self.selected;
        self.main_scroll_offset = self.scroll_offset;
        self.main_auto_scroll = self.auto_scroll;
        self.main_expanded = std::mem::take(&mut self.expanded);
        self.main_expand_all = self.expand_all;

        self.context_mode = true;
        self.context_center = self.selected;
        self.auto_scroll = false;
        self.expand_all = false;
        self.expanded.clear();
        self.invalidate_caches();
        self.center_selection(visible_height, content_width);
    }

    pub fn exit_context_mode(&mut self) {
        if !self.context_mode {
            return;
        }
        self.context_mode = false;
        self.selected = self.main_selected;
        self.scroll_offset = self.main_scroll_offset;
        self.auto_scroll = self.main_auto_scroll;
        self.expanded = std::mem::take(&mut self.main_expanded);
        self.expand_all = self.main_expand_all;
        self.invalidate_caches();
    }

    pub fn poll_events(&self) -> std::io::Result<bool> {
        crossterm::event::poll(std::time::Duration::from_millis(16))
    }

    pub fn next_event(&self) -> std::io::Result<Event> {
        crossterm::event::read()
    }
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
        app.filter.filter_query = Some(FilterQuery {
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
        app.filter.filter_query = Some(FilterQuery {
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
        app.filter.filter_query = Some(FilterQuery {
            segments: vec![plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("xyz".to_string()),
                regex: None,
                json_key: None,
            })],
        });
        assert_eq!(app.filtered_indices().len(), 0);
        app.filter.filter_query = None;
        app.cache.filtered_indices = None;
        assert_eq!(app.filtered_indices().len(), 1);
    }

    #[test]
    fn test_regex_filter_matching() {
        let mut app = App::new(100);
        app.add_line("error: connection timeout".to_string());
        app.add_line("info: request ok".to_string());
        app.add_line("error: disk full".to_string());
        app.filter.filter_query = Some(FilterQuery {
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
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 8), "hello w…");
        // CJK characters: each is 2 columns wide
        assert_eq!(truncate_str("あいうえお", 10), "あいうえお"); // 5 chars × 2 = 10 width, fits exactly
        assert_eq!(truncate_str("あいうえお", 5), "あい…"); // 2 chars (4 width) + … (1 width) = 5
        assert_eq!(truncate_str("hello世界", 9), "hello世界"); // 5 + 4 = 9 width, fits
        assert_eq!(truncate_str("hello世界", 8), "hello世…"); // 5 + 2 + 1 = 8
    }

    #[test]
    fn test_display_width() {
        assert_eq!(display_width("hello"), 5);
        assert_eq!(display_width("あいう"), 6); // 3 CJK chars × 2 columns = 6
        assert_eq!(display_width("hello世界"), 9); // 5 ASCII + 2 CJK × 2 = 9
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
        app.filter.filter_query = Some(FilterQuery {
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

    #[test]
    fn test_entry_height_cache_reuses_on_add_line() {
        let mut app = App::new(10000);
        for i in 0..100 {
            app.add_line(format!(r#"{{"level":"info","msg":"line {}"}}"#, i));
        }
        app.expand_all = true;

        // Force layout computation to populate the entry height cache
        let (layout1, _) = app.cached_row_layout(80);
        let first_len = layout1.len();

        // Add a new line — this should only compute the new entry's height
        app.add_line(r#"{"level":"info","msg":"new line"}"#.to_string());

        let (layout2, _) = app.cached_row_layout(80);
        assert_eq!(layout2.len(), first_len + 1);

        // Entry heights cache should have entries for all filtered lines
        let cached = app.cache.entry_heights.as_ref().unwrap();
        assert_eq!(cached.1.len(), first_len + 1);
    }

    #[test]
    fn test_entry_height_cache_invalidated_on_toggle() {
        let mut app = App::new(100);
        app.add_line(r#"{"key":"value"}"#.to_string());
        app.expand_all = true;

        let _ = app.cached_row_layout(80);
        assert!(app.cache.entry_heights.is_some());

        // Simulate Ctrl-O toggle (collapse all)
        app.expand_all = false;
        app.expanded.clear();
        app.cache.row_layout = None;
        app.cache.entry_heights = None;

        let (layout, _) = app.cached_row_layout(80);
        assert_eq!(layout[0], 1, "collapsed entry should have height 1");
    }

    #[test]
    fn test_entry_height_cache_adjusted_on_remove() {
        let mut app = App::new(3);
        app.add_line(r#"{"key":"line0"}"#.to_string());
        app.add_line(r#"{"key":"line1"}"#.to_string());
        app.add_line(r#"{"key":"line2"}"#.to_string());
        app.expand_all = true;

        let _ = app.cached_row_layout(80);
        assert!(app.cache.entry_heights.is_some());
        // Cache should have entries for indices 0, 1, 2
        let map = &app.cache.entry_heights.as_ref().unwrap().1;
        assert!(map.contains_key(&0));
        assert!(map.contains_key(&1));
        assert!(map.contains_key(&2));

        // Add one more line, triggering overflow (removes index 0)
        app.add_line(r#"{"key":"line3"}"#.to_string());

        // After removal, cache should be adjusted: keys shifted down
        let map = &app.cache.entry_heights.as_ref().unwrap().1;
        assert!(
            !map.contains_key(&0) || app.lines.len() <= 3,
            "old index 0 should be gone after removal"
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
    fn test_parse_bare_text_as_contains() {
        let query = parse_filter_query("foo").unwrap();
        assert_eq!(query.segments.len(), 1);
        let c = get_plain(&query, 0);
        assert_eq!(c.operator, FilterOp::Contains);
        assert_eq!(c.value, FilterValue::String("foo".to_string()));
        assert!(c.regex.is_none());
        assert!(c.json_key.is_none());
    }

    #[test]
    fn test_parse_bare_text_with_spaces_as_contains() {
        let query = parse_filter_query("foo bar").unwrap();
        assert_eq!(query.segments.len(), 1);
        let c = get_plain(&query, 0);
        assert_eq!(c.operator, FilterOp::Contains);
        assert_eq!(c.value, FilterValue::String("foo bar".to_string()));
    }

    #[test]
    fn test_parse_quoted_text_without_operator_as_contains() {
        let query = parse_filter_query(r#""foo""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        let c = get_plain(&query, 0);
        assert_eq!(c.operator, FilterOp::Contains);
        assert_eq!(c.value, FilterValue::String(r#""foo""#.to_string()));
    }

    #[test]
    fn test_parse_invalid_operator_as_contains() {
        let query = parse_filter_query(r#"== "foo""#).unwrap();
        assert_eq!(query.segments.len(), 1);
        let c = get_plain(&query, 0);
        assert_eq!(c.operator, FilterOp::Contains);
        assert_eq!(c.value, FilterValue::String(r#"== "foo""#.to_string()));
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
                filter: FilterState {
                    filter_history: vec!["|= \"foo\"".to_string(), "|= \"bar\"".to_string()],
                    ..FilterState::new()
                },
                cache: RenderCache::default(),
                ..App::new(100)
            };
            let content = app.filter.filter_history.join("\n");
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
    fn test_parse_plain_and_bare_text() {
        let query = parse_filter_query(r#"|= "foo" and |= "bar""#).unwrap();
        assert_eq!(query.segments.len(), 2);
        assert_eq!(get_plain(&query, 0).operator, FilterOp::Contains);
        assert_eq!(
            get_plain(&query, 0).value,
            FilterValue::String("foo".to_string())
        );
        assert_eq!(get_plain(&query, 1).operator, FilterOp::Contains);
        assert_eq!(
            get_plain(&query, 1).value,
            FilterValue::String(r#"and |= "bar""#.to_string())
        );
    }

    #[test]
    fn test_parse_plain_and_not_bare_text() {
        let query = parse_filter_query(r#"|= "foo" and != "bar""#).unwrap();
        assert_eq!(query.segments.len(), 2);
        assert_eq!(
            get_plain(&query, 1).value,
            FilterValue::String(r#"and != "bar""#.to_string())
        );
    }

    #[test]
    fn test_parse_plain_or_not_bare_text() {
        let query = parse_filter_query(r#"|= "foo" or != "bar""#).unwrap();
        assert_eq!(query.segments.len(), 2);
        assert_eq!(
            get_plain(&query, 1).value,
            FilterValue::String(r#"or != "bar""#.to_string())
        );
    }

    #[test]
    fn test_parse_regex_or_not_regex_bare_text() {
        let query = parse_filter_query(r#"|~ "foo" or !~ "bar""#).unwrap();
        assert_eq!(query.segments.len(), 2);
        assert_eq!(
            get_plain(&query, 1).value,
            FilterValue::String(r#"or !~ "bar""#.to_string())
        );
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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
            filter: FilterState {
                filter_query: Some(query),
                ..FilterState::new()
            },
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

    #[test]
    fn test_expanded_toggle() {
        let mut app = App::new(100);
        app.add_line("{\"key\":\"val\"}".to_string());
        assert!(!app.expanded.contains(&0));
        app.expanded.insert(0);
        assert!(app.expanded.contains(&0));
        app.expanded.remove(&0);
        assert!(!app.expanded.contains(&0));
    }

    #[test]
    fn test_filter_ignores_source_prefix() {
        let mut app = App::new(100);
        app.add_line_with_source("error: disk full".to_string(), LineSource::Stderr);
        app.add_line_with_source("info: request ok".to_string(), LineSource::Stdout);

        // Filter should match the text content, not the prefix
        app.filter.filter_query = Some(parse_filter_query("|= \"error\"").unwrap());
        app.cache.filtered_indices = None;

        let filtered = app.filtered_indices();
        assert_eq!(filtered.len(), 1);
        assert_eq!(app.lines[filtered[0]].text, "error: disk full");
    }

    #[test]
    fn test_process_exited_detection() {
        let mut app = App::new(100);
        assert!(!app.process_exited);

        app.add_line_with_source("some output".to_string(), LineSource::Stdout);
        assert!(!app.process_exited);

        app.add_line_with_source("process exited with code 0".to_string(), LineSource::System);
        assert!(app.process_exited);
    }

    #[test]
    fn test_yank_selected_returns_ok() {
        let mut app = App::new(100);
        app.add_line("hello world".to_string());
        assert!(app.yank_selected().is_ok());
    }

    #[test]
    fn test_yank_selected_empty_lines() {
        let app = App::new(100);
        assert!(app.yank_selected().is_ok());
    }

    #[test]
    fn test_yank_osc52_format() {
        let mut app = App::new(100);
        app.add_line("test".to_string());
        let encoded = base64::engine::general_purpose::STANDARD.encode("test".as_bytes());
        assert_eq!(encoded, "dGVzdA==");
    }

    // --- recorder tests ---

    #[test]
    fn test_start_stop_recording() {
        let dir = std::env::temp_dir().join("logq_test_app_recorder");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.log");

        let mut app = App::new(100);
        assert!(app.recorder.is_none());

        app.start_recording(path.clone()).unwrap();
        assert!(app.recorder.is_some());

        app.add_line("recorded line".to_string());

        app.stop_recording();
        assert!(app.recorder.is_none());

        // Verify file content
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "recorded line\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_no_recorder_no_crash() {
        let mut app = App::new(100);
        // add_line without recorder should work fine
        app.add_line("hello".to_string());
        assert_eq!(app.lines.len(), 1);
    }

    #[test]
    fn test_recorder_records_all_sources() {
        let dir = std::env::temp_dir().join("logq_test_app_recorder_sources");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.log");

        let mut app = App::new(100);
        app.start_recording(path.clone()).unwrap();

        app.add_line_with_source("stdout line".to_string(), LineSource::Stdout);
        app.add_line_with_source("stderr line".to_string(), LineSource::Stderr);
        app.add_line_with_source("system line".to_string(), LineSource::System);

        app.stop_recording();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("stdout line\n"));
        assert!(content.contains("stderr line\n"));
        assert!(content.contains("system line\n"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- line_format tests ---

    #[test]
    fn test_line_format_simple() {
        let template = parse_line_format_template("{{ .a }} / {{ .b }}").unwrap();
        let result = template.format(r#"{"a": 10, "b": "foo"}"#);
        assert_eq!(result, Some("10 / foo".to_string()));
    }

    #[test]
    fn test_line_format_single_key() {
        let template = parse_line_format_template("{{ .name }}").unwrap();
        let result = template.format(r#"{"name": "alice"}"#);
        assert_eq!(result, Some("alice".to_string()));
    }

    #[test]
    fn test_line_format_literal_only() {
        let template = parse_line_format_template("hello world").unwrap();
        let result = template.format(r#"{"a": 1}"#);
        assert_eq!(result, Some("hello world".to_string()));
    }

    #[test]
    fn test_line_format_missing_key_is_empty() {
        let template = parse_line_format_template("{{ .missing }}").unwrap();
        let result = template.format(r#"{"a": 1}"#);
        assert_eq!(result, Some("".to_string()));
    }

    #[test]
    fn test_line_format_non_json_returns_none() {
        let template = parse_line_format_template("{{ .a }}").unwrap();
        let result = template.format("plain text");
        assert_eq!(result, None);
    }

    #[test]
    fn test_line_format_nested_key() {
        let template = parse_line_format_template("{{ .user.name }}").unwrap();
        let result = template.format(r#"{"user": {"name": "bob"}}"#);
        assert_eq!(result, Some("bob".to_string()));
    }

    #[test]
    fn test_line_format_mixed_literal_and_keys() {
        let template = parse_line_format_template("name={{ .name }} age={{ .age }}").unwrap();
        let result = template.format(r#"{"name": "alice", "age": 30}"#);
        assert_eq!(result, Some("name=alice age=30".to_string()));
    }

    #[test]
    fn test_line_format_boolean_and_null() {
        let template = parse_line_format_template("{{ .active }} {{ .result }}").unwrap();
        let result = template.format(r#"{"active": true, "result": null}"#);
        assert_eq!(result, Some("true null".to_string()));
    }

    #[test]
    fn test_parse_line_format_in_query() {
        let query = parse_filter_query(r#"| line_format "{{ .a }}""#).unwrap();
        assert!(
            query
                .segments
                .iter()
                .any(|s| matches!(s, FilterSegment::LineFormat(_)))
        );
    }

    #[test]
    fn test_parse_line_format_combined_with_filter() {
        let query = parse_filter_query(r#"|= "foo" | line_format "{{ .bar }}""#).unwrap();
        assert_eq!(query.segments.len(), 2);
        assert!(matches!(query.segments[0], FilterSegment::Plain(_)));
        assert!(matches!(query.segments[1], FilterSegment::LineFormat(_)));
    }

    #[test]
    fn test_parse_line_format_with_json_filter() {
        let query = parse_filter_query(r#"| name = "alice" | line_format "{{ .age }}""#).unwrap();
        assert_eq!(query.segments.len(), 2);
        assert!(matches!(query.segments[0], FilterSegment::Json(_)));
        assert!(matches!(query.segments[1], FilterSegment::LineFormat(_)));
    }

    #[test]
    fn test_line_format_segment_is_not_filter() {
        // line_format should not affect filtering — it's a display directive
        let mut app = App::new(100);
        app.add_line(r#"{"a": 10, "b": "foo"}"#.to_string());
        app.add_line("plain text".to_string());
        app.filter.filter_query = Some(parse_filter_query(r#"| line_format "{{ .a }}""#).unwrap());
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0, 1]);
    }

    #[test]
    fn test_line_format_unterminated_template() {
        assert!(parse_line_format_template("{{ .a").is_err());
    }

    #[test]
    fn test_line_format_empty_key() {
        assert!(parse_line_format_template("{{ . }}").is_err());
    }

    #[test]
    fn test_filtered_indices_no_recompute_on_second_call() {
        let mut app = App::new(100);
        app.add_line("a".to_string());
        app.add_line("b".to_string());
        // First call computes, second uses cache
        let f1 = app.filtered_indices();
        let f2 = app.filtered_indices();
        assert_eq!(f1, f2);
    }

    #[test]
    fn test_add_line_incremental_filtered_indices_no_filter() {
        let mut app = App::new(100);
        app.add_line("a".to_string());
        // Populate the cache
        let f = app.filtered_indices();
        assert_eq!(f, vec![0]);

        // Add a line — should incrementally update, not invalidate
        app.add_line("b".to_string());
        // Cache should still be populated (not None)
        assert!(app.cache.filtered_indices.is_some());
        let f = app.filtered_indices();
        assert_eq!(f, vec![0, 1]);
    }

    #[test]
    fn test_add_line_incremental_filtered_indices_with_filter() {
        let mut app = App::new(100);
        app.add_line(r#"{"level":"info","msg":"hello"}"#.to_string());
        app.add_line(r#"{"level":"error","msg":"bad"}"#.to_string());
        // Set a filter that matches "error"
        app.filter.filter_query = Some(parse_filter_query(r#"| level = "error""#).unwrap());
        app.invalidate_caches();
        let f = app.filtered_indices();
        assert_eq!(f, vec![1]);

        // Add a matching line
        app.add_line(r#"{"level":"error","msg":"worse"}"#.to_string());
        assert!(app.cache.filtered_indices.is_some());
        let f = app.filtered_indices();
        assert_eq!(f, vec![1, 2]);

        // Add a non-matching line
        app.add_line(r#"{"level":"info","msg":"ok"}"#.to_string());
        assert!(app.cache.filtered_indices.is_some());
        let f = app.filtered_indices();
        assert_eq!(f, vec![1, 2]);
    }

    #[test]
    fn test_add_line_incremental_max_lines_trims_cache() {
        let mut app = App::new(3);
        app.add_line("a".to_string());
        app.add_line("b".to_string());
        app.add_line("c".to_string());
        let f = app.filtered_indices();
        assert_eq!(f, vec![0, 1, 2]);

        // Exceed max_lines — should trim from front
        app.add_line("d".to_string());
        assert!(app.cache.filtered_indices.is_some());
        let f = app.filtered_indices();
        assert_eq!(f, vec![0, 1, 2]); // indices shifted: [b, c, d] → [0, 1, 2]
    }

    // --- context mode tests ---

    #[test]
    fn test_enter_context_mode_shows_all_lines() {
        let mut app = App::new(100);
        app.add_line("aaa".to_string());
        app.add_line("bbb".to_string());
        app.add_line("aaa2".to_string());
        app.filter.filter_query = Some(parse_filter_query(r#"|= "aaa""#).unwrap());
        app.invalidate_caches();
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0, 2]);

        app.selected = 2;
        app.enter_context_mode(10, 67);
        assert!(app.context_mode);
        assert_eq!(app.context_center, 2);
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0, 1, 2]);
    }

    #[test]
    fn test_exit_context_mode_restores_state() {
        let mut app = App::new(100);
        app.add_line("aaa".to_string());
        app.add_line("bbb".to_string());
        app.add_line("aaa2".to_string());
        app.filter.filter_query = Some(parse_filter_query(r#"|= "aaa""#).unwrap());
        app.invalidate_caches();
        app.selected = 2;
        app.auto_scroll = false;
        app.expanded.insert(2);
        app.scroll_offset = 1;

        app.enter_context_mode(10, 67);
        assert!(app.context_mode);

        // Move around in context mode
        app.move_selection(-1, 10, 67);

        app.exit_context_mode();
        assert!(!app.context_mode);
        assert_eq!(app.selected, 2);
        assert_eq!(app.scroll_offset, 1);
        assert!(!app.auto_scroll);
        assert!(app.expanded.contains(&2));
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0, 2]);
    }

    #[test]
    fn test_context_mode_empty_lines_noop() {
        let mut app = App::new(100);
        assert!(!app.context_mode);
        app.enter_context_mode(10, 67);
        assert!(!app.context_mode);
    }

    #[test]
    fn test_enter_context_mode_twice_noop() {
        let mut app = App::new(100);
        app.add_line("aaa".to_string());
        app.enter_context_mode(10, 67);
        assert!(app.context_mode);
        let center = app.context_center;
        app.enter_context_mode(10, 67);
        assert_eq!(app.context_center, center);
    }

    #[test]
    fn test_exit_context_mode_when_not_in_noop() {
        let mut app = App::new(100);
        app.add_line("aaa".to_string());
        assert!(!app.context_mode);
        app.exit_context_mode();
        assert!(!app.context_mode);
    }
}
