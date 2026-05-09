use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{App, TIMESTAMP_WIDTH};

impl App {
    pub fn handle_event(&mut self, event: Event, area: ratatui::layout::Rect) -> bool {
        match event {
            Event::Resize(_, _) => true,
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return false;
                }

                if self.show_help {
                    self.handle_help_key(key);
                    return true;
                }

                let visible_height = self.visible_height(&area);
                let content_width = (area.width as usize).saturating_sub(TIMESTAMP_WIDTH);

                // Handle command input mode
                if self.command_input.is_some() {
                    self.handle_command_input(key);
                    return true;
                }

                // Handle filter input mode
                if self.filter.filter_input.is_some() {
                    self.handle_filter_input(key);
                    return true;
                }

                self.handle_list_key(key.code, key.modifiers, visible_height, content_width);
                true
            }
            _ => false,
        }
    }

    pub fn handle_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.show_help = false,
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.show_help = false
            }
            KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.handle_ctrl_x();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.help_scroll = self.help_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
            }
            _ => {}
        }
    }

    pub fn handle_filter_input(&mut self, key: KeyEvent) {
        let (invalidated, toggle_help, quit) = self.filter.handle_filter_key(key);
        if quit {
            self.should_quit = true;
        }
        if toggle_help {
            self.help_scroll = 0;
            self.show_help = true;
        }
        if invalidated {
            self.invalidate_caches();
        }
        // After submit, clamp selection to filtered set
        if self.filter.filter_input.is_none() {
            let filtered = self.filtered_indices();
            if !filtered.is_empty() {
                if self.selected > filtered[filtered.len() - 1] {
                    self.selected = filtered[filtered.len() - 1];
                } else if !filtered.contains(&self.selected) {
                    self.selected = filtered[0];
                }
            }
        }
    }

    pub fn handle_list_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        visible_height: usize,
        content_width: usize,
    ) {
        let filtered = self.filtered_indices();
        let max_idx = filtered.len().saturating_sub(1);

        // Hoist: any keypress except a g-prefix continuation clears the pending state.
        let prev_pending_g = self.pending_g;
        self.pending_g = false;

        let prev_pending_z = self.pending_z;
        self.pending_z = false;

        match (code, modifiers) {
            (KeyCode::Char('x'), KeyModifiers::CONTROL) => {
                self.handle_ctrl_x();
            }
            (KeyCode::Char('j'), _) | (KeyCode::Down, _)
                if !self.try_scroll_expanded(true, visible_height, content_width) =>
            {
                self.move_selection(1, visible_height, content_width);
            }
            (KeyCode::Char('k'), _) | (KeyCode::Up, _)
                if !self.try_scroll_expanded(false, visible_height, content_width) =>
            {
                self.move_selection(-1, visible_height, content_width);
            }
            (KeyCode::Char('G'), _) if !filtered.is_empty() => {
                self.selected = filtered[max_idx];
                self.auto_scroll = true;
                self.ensure_selection_visible(visible_height, content_width);
            }
            (KeyCode::Char('g'), KeyModifiers::CONTROL) => {
                self.help_scroll = 0;
                self.show_help = true;
            }
            (KeyCode::Char('g'), _) if !filtered.is_empty() => {
                if prev_pending_g {
                    self.selected = filtered[0];
                    self.auto_scroll = false;
                    self.scroll_offset = 0;
                } else {
                    self.pending_g = true;
                }
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                let half = (visible_height / 2).max(1);
                self.page_move(half as isize, visible_height, content_width, true);
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                let half = (visible_height / 2).max(1);
                self.page_move(-(half as isize), visible_height, content_width, false);
            }
            (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                self.page_move(visible_height as isize, visible_height, content_width, true);
            }
            (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                self.page_move(
                    -(visible_height as isize),
                    visible_height,
                    content_width,
                    false,
                );
            }
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                let (_, prefix_sums) = self.cached_row_layout(content_width);
                let total_rows = *prefix_sums.last().unwrap_or(&0);
                let max_offset = total_rows.saturating_sub(visible_height);
                self.scroll_offset = (self.scroll_offset + 1).min(max_offset);
                self.auto_scroll = false;
            }
            (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                self.auto_scroll = false;
            }
            (KeyCode::Enter, _) if !filtered.is_empty() => {
                if self.expanded.contains(&self.selected) {
                    self.expanded.remove(&self.selected);
                } else {
                    self.expanded.insert(self.selected);
                }
                self.cache.row_layout = None;
                self.cache.entry_heights = None;
            }
            (KeyCode::Char('o'), KeyModifiers::CONTROL) => {
                if self.expand_all {
                    self.expand_all = false;
                    self.expanded.clear();
                } else {
                    self.expand_all = true;
                    for &idx in &filtered {
                        self.expanded.insert(idx);
                    }
                }
                self.cache.row_layout = None;
                self.cache.entry_heights = None;
            }
            (KeyCode::Char('/'), _) if !self.context_mode => {
                self.filter.start_filter_input();
                self.filter.update_live_filter();
            }
            (KeyCode::Char(':'), _) if !self.context_mode => {
                self.command_input = Some(tui_input::Input::default());
                self.command_error = None;
            }
            (KeyCode::Char('c'), _) if !self.context_mode => {
                self.enter_context_mode(visible_height, content_width);
            }
            (KeyCode::Esc, _) => {
                if self.context_mode {
                    self.exit_context_mode();
                } else {
                    self.filter.filter_query = None;
                    self.invalidate_caches();
                }
            }
            (KeyCode::Char('y'), _) => {
                let _ = self.yank_selected();
            }
            (KeyCode::Char('z'), _) => {
                if prev_pending_z {
                    self.center_selection(visible_height, content_width);
                } else {
                    self.pending_z = true;
                }
            }
            _ => {}
        }
    }

    pub fn handle_ctrl_x(&mut self) {
        self.should_quit = true;
    }

    pub fn handle_command_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                let input = self.command_input.take();
                if let Some(input) = input {
                    let value = input.value().trim().to_string();
                    self.execute_command(&value);
                }
            }
            KeyCode::Esc => {
                self.command_input = None;
                self.command_error = None;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.command_input = None;
                self.command_error = None;
            }
            KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            _ => {
                if let Some(input) = &mut self.command_input {
                    use tui_input::backend::crossterm::EventHandler;
                    if input.handle_event(&Event::Key(key)).is_some() {
                        self.command_error = None;
                    }
                }
            }
        }
    }

    fn execute_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.splitn(2, char::is_whitespace).collect();
        match parts.as_slice() {
            ["record", path] => {
                let path = path.trim();
                if path.is_empty() {
                    self.command_error = Some(t!("command.error.missing_path").to_string());
                    return;
                }
                if let Err(e) = self.start_recording(std::path::PathBuf::from(path)) {
                    self.command_error =
                        Some(t!("command.error.record_failed", error = e.to_string()).to_string());
                }
            }
            ["stoprecord"] | ["stop"] => {
                self.stop_recording();
            }
            [""] => {}
            _ => {
                self.command_error = Some(t!("command.error.unknown", cmd = cmd).to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use crate::app::App;

    #[test]
    fn test_ctrl_x_quit() {
        let mut app = App::new(100);
        app.handle_ctrl_x();
        assert!(app.should_quit);
    }

    #[test]
    fn test_ctrl_x_quit_from_help() {
        let mut app = App::new(100);
        app.show_help = true;
        app.handle_help_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn test_ctrl_x_quit_from_filter_input() {
        let mut app = App::new(100);
        app.filter.filter_input = Some(tui_input::Input::new("test".to_string()));
        app.handle_filter_input(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn test_ctrl_g_opens_help() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        assert!(!app.show_help);
        app.handle_list_key(KeyCode::Char('g'), KeyModifiers::CONTROL, 24, 67);
        assert!(app.show_help);
    }

    #[test]
    fn test_help_esc_closes() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        app.show_help = true;
        app.handle_help_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.show_help);
    }

    #[test]
    fn test_help_ctrl_g_closes() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        app.show_help = true;
        app.handle_help_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
        assert!(!app.show_help);
    }

    #[test]
    fn test_help_ignores_other_keys() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        app.show_help = true;
        app.handle_help_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(app.show_help);
    }

    #[test]
    fn test_handle_event_key_returns_true() {
        let mut app = App::new(100);
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let event = Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(app.handle_event(event, area));
    }

    #[test]
    fn test_handle_event_mouse_returns_false() {
        let mut app = App::new(100);
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let event = Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Moved,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert!(!app.handle_event(event, area));
    }

    #[test]
    fn test_handle_event_resize_returns_true() {
        let mut app = App::new(100);
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let event = Event::Resize(120, 40);
        assert!(app.handle_event(event, area));
    }

    #[test]
    fn test_handle_event_key_release_returns_false() {
        let mut app = App::new(100);
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let event = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ));
        assert!(!app.handle_event(event, area));
    }

    #[test]
    fn test_gg_jumps_to_first_line() {
        let mut app = App::new(100);
        for i in 0..20 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 15;
        app.scroll_offset = 10;
        app.auto_scroll = false;

        // First 'g' press: pending_g becomes true
        app.handle_list_key(KeyCode::Char('g'), KeyModifiers::NONE, 10, 67);
        assert!(app.pending_g);
        assert_eq!(app.selected, 15); // no movement yet

        // Second 'g' press: jumps to first line
        app.handle_list_key(KeyCode::Char('g'), KeyModifiers::NONE, 10, 67);
        assert!(!app.pending_g);
        assert_eq!(app.selected, 0);
        assert_eq!(app.scroll_offset, 0);
        assert!(!app.auto_scroll);
    }

    #[test]
    fn test_pending_g_resets_on_other_key() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 5;

        // First 'g' press
        app.handle_list_key(KeyCode::Char('g'), KeyModifiers::NONE, 10, 67);
        assert!(app.pending_g);

        // Press 'j': should reset pending_g and move normally
        app.handle_list_key(KeyCode::Char('j'), KeyModifiers::NONE, 10, 67);
        assert!(!app.pending_g);
        assert_eq!(app.selected, 6); // moved by 1, not jumped to first
    }

    #[test]
    fn test_pending_g_resets_on_unknown_key() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 5;

        app.handle_list_key(KeyCode::Char('g'), KeyModifiers::NONE, 10, 67);
        assert!(app.pending_g);

        app.handle_list_key(KeyCode::Char('z'), KeyModifiers::NONE, 10, 67);
        assert!(!app.pending_g);
        assert_eq!(app.selected, 5); // no movement
    }

    #[test]
    fn test_j_scrolls_within_expanded_entry() {
        // Create a long JSON object that wraps to many lines in a narrow viewport
        let mut app = App::new(100);
        let fields: Vec<String> = (0..30)
            .map(|i| format!(r#""key{i}": "value{i}""#))
            .collect();
        let long_json = format!("{{{}}}", fields.join(", "));
        app.add_line(long_json.clone());
        app.add_line("short".to_string());

        let visible_height = 5usize;
        let content_width = 20usize;

        // Expand first entry
        app.selected = 0;
        app.expanded.insert(0);
        app.cache.row_layout = None;
        app.cache.entry_heights = None;

        // Compute layout to determine entry height
        let (_, prefix_sums) = app.cached_row_layout(content_width);
        let entry_height = prefix_sums[1] - prefix_sums[0];
        assert!(
            entry_height > visible_height,
            "entry must be taller than viewport"
        );

        // Initial state: scroll_offset at 0, viewport shows first rows
        app.scroll_offset = 0;
        app.auto_scroll = false;

        // Press j — should scroll within entry, NOT move to next entry
        app.handle_list_key(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            visible_height,
            content_width,
        );
        assert_eq!(app.selected, 0, "selected should stay on expanded entry");
        assert_eq!(app.scroll_offset, 1, "scroll_offset should increase by 1");

        // Press j again — still scrolling within
        app.handle_list_key(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            visible_height,
            content_width,
        );
        assert_eq!(
            app.selected, 0,
            "selected should still stay on expanded entry"
        );
        assert_eq!(
            app.scroll_offset, 2,
            "scroll_offset should increase by 1 again"
        );
    }

    #[test]
    fn test_j_moves_to_next_entry_at_bottom_boundary() {
        let mut app = App::new(100);
        let fields: Vec<String> = (0..30)
            .map(|i| format!(r#""key{i}": "value{i}""#))
            .collect();
        let long_json = format!("{{{}}}", fields.join(", "));
        app.add_line(long_json.clone());
        app.add_line("short".to_string());

        let visible_height = 5usize;
        let content_width = 20usize;

        app.selected = 0;
        app.expanded.insert(0);
        app.cache.row_layout = None;
        app.cache.entry_heights = None;

        let (_, prefix_sums) = app.cached_row_layout(content_width);
        let entry_height = prefix_sums[1] - prefix_sums[0];

        // Set scroll_offset to the bottom of the entry
        app.scroll_offset = entry_height.saturating_sub(visible_height);
        app.auto_scroll = false;

        // Press j — should move to next entry
        app.handle_list_key(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            visible_height,
            content_width,
        );
        assert_eq!(app.selected, 1, "selected should move to next entry");
    }

    #[test]
    fn test_k_scrolls_up_within_expanded_entry() {
        let mut app = App::new(100);
        let fields: Vec<String> = (0..30)
            .map(|i| format!(r#""key{i}": "value{i}""#))
            .collect();
        let long_json = format!("{{{}}}", fields.join(", "));
        app.add_line(long_json.clone());
        app.add_line("short".to_string());

        let visible_height = 5usize;
        let content_width = 20usize;

        app.selected = 0;
        app.expanded.insert(0);
        app.cache.row_layout = None;
        app.cache.entry_heights = None;

        // Set scroll_offset past the start of the entry
        app.scroll_offset = 3;
        app.auto_scroll = false;

        // Press k — should scroll up within entry
        app.handle_list_key(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
            visible_height,
            content_width,
        );
        assert_eq!(app.selected, 0, "selected should stay on expanded entry");
        assert_eq!(app.scroll_offset, 2, "scroll_offset should decrease by 1");
    }

    #[test]
    fn test_k_moves_to_prev_entry_at_top_boundary() {
        let mut app = App::new(100);
        let fields: Vec<String> = (0..30)
            .map(|i| format!(r#""key{i}": "value{i}""#))
            .collect();
        let long_json = format!("{{{}}}", fields.join(", "));
        app.add_line("first".to_string());
        app.add_line(long_json.clone());

        let visible_height = 5usize;
        let content_width = 20usize;

        app.selected = 1;
        app.expanded.insert(1);
        app.cache.row_layout = None;
        app.cache.entry_heights = None;

        let (_, prefix_sums) = app.cached_row_layout(content_width);
        let entry_first_row = prefix_sums[1];

        // scroll_offset at the start of the entry
        app.scroll_offset = entry_first_row;
        app.auto_scroll = false;

        // Press k — should move to previous entry
        app.handle_list_key(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
            visible_height,
            content_width,
        );
        assert_eq!(app.selected, 0, "selected should move to previous entry");
    }

    #[test]
    fn test_ctrl_e_y_viewport_scroll_without_selection_change() {
        let mut app = App::new(100);
        let fields: Vec<String> = (0..30)
            .map(|i| format!(r#""key{i}": "value{i}""#))
            .collect();
        let long_json = format!("{{{}}}", fields.join(", "));
        app.add_line(long_json.clone());
        app.add_line("short".to_string());

        let visible_height = 5usize;
        let content_width = 20usize;

        app.selected = 0;
        app.expanded.insert(0);
        app.cache.row_layout = None;
        app.cache.entry_heights = None;
        app.scroll_offset = 0;
        app.auto_scroll = false;

        // Ctrl-e: scroll viewport down
        app.handle_list_key(
            KeyCode::Char('e'),
            KeyModifiers::CONTROL,
            visible_height,
            content_width,
        );
        assert_eq!(app.selected, 0, "Ctrl-e should not change selection");
        assert_eq!(app.scroll_offset, 1, "Ctrl-e should scroll down by 1");

        // Ctrl-e again
        app.handle_list_key(
            KeyCode::Char('e'),
            KeyModifiers::CONTROL,
            visible_height,
            content_width,
        );
        assert_eq!(app.selected, 0, "Ctrl-e should not change selection");
        assert_eq!(app.scroll_offset, 2, "Ctrl-e should scroll down by 1 again");

        // Ctrl-y: scroll viewport up
        app.handle_list_key(
            KeyCode::Char('y'),
            KeyModifiers::CONTROL,
            visible_height,
            content_width,
        );
        assert_eq!(app.selected, 0, "Ctrl-y should not change selection");
        assert_eq!(app.scroll_offset, 1, "Ctrl-y should scroll up by 1");
    }

    #[test]
    fn test_j_k_collapsed_entry_moves_normally() {
        let mut app = App::new(100);
        app.add_line("line 0".to_string());
        app.add_line("line 1".to_string());
        app.add_line("line 2".to_string());

        let visible_height = 10usize;
        let content_width = 40usize;

        app.selected = 1;
        app.auto_scroll = false;

        // j on collapsed entry should move to next entry
        app.handle_list_key(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            visible_height,
            content_width,
        );
        assert_eq!(app.selected, 2);

        // k on collapsed entry should move to prev entry
        app.handle_list_key(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
            visible_height,
            content_width,
        );
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn test_slash_prepopulates_current_filter() {
        use crate::filter::parse_filter_query;
        let mut app = App::new(100);
        app.add_line("foo line".to_string());
        app.add_line("bar line".to_string());

        // Set a filter query directly (simulates an already-applied filter)
        app.filter.filter_query = Some(parse_filter_query(r#"|= "foo""#).unwrap());
        assert!(app.filter.filter_query.is_some());
        assert!(app.filter.filter_input.is_none());

        // Press / — should pre-populate with current filter
        app.handle_list_key(KeyCode::Char('/'), KeyModifiers::NONE, 10, 80);
        assert!(
            app.filter.filter_input.is_some(),
            "filter_input should be set after pressing /"
        );
        let input_value = app
            .filter
            .filter_input
            .as_ref()
            .unwrap()
            .value()
            .to_string();
        assert_eq!(
            input_value, r#"|= "foo""#,
            "filter input should be pre-populated with current filter query"
        );
    }

    #[test]
    fn test_slash_starts_empty_when_no_filter() {
        let mut app = App::new(100);
        app.add_line("foo line".to_string());

        assert!(app.filter.filter_query.is_none());

        // Press / — should start with empty input
        app.handle_list_key(KeyCode::Char('/'), KeyModifiers::NONE, 10, 80);
        assert!(app.filter.filter_input.is_some());
        assert_eq!(
            app.filter.filter_input.as_ref().unwrap().value(),
            "",
            "filter input should be empty when no filter is active"
        );
    }

    #[test]
    fn test_live_filter_keeps_previous_valid_on_parse_error() {
        let mut app = App::new(100);
        app.add_line("foo line".to_string());
        app.add_line("bar line".to_string());
        app.add_line("foo bar line".to_string());

        // Start filter input mode with a valid query
        app.filter.filter_input = Some(tui_input::Input::new(r#"|= "foo""#.to_string()));
        app.filter.update_live_filter();
        app.invalidate_caches();
        assert!(app.filter.live_filter_query.is_some());
        assert!(app.filter.live_filter_error.is_none());
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0, 2]);

        // Add invalid suffix: parse error, but previous valid query should be kept
        app.filter.filter_input = Some(tui_input::Input::new(r#"|= "foo" |"#.to_string()));
        app.filter.update_live_filter();
        assert!(
            app.filter.live_filter_query.is_some(),
            "live_filter_query should keep previous valid query on parse error"
        );
        assert!(app.filter.live_filter_error.is_some());
        let filtered = app.filtered_indices();
        assert_eq!(
            filtered,
            vec![0, 2],
            "should still filter by previous valid query"
        );

        // Continue typing to make it valid again with two conditions
        app.filter.filter_input = Some(tui_input::Input::new(r#"|= "foo" |= "bar""#.to_string()));
        app.filter.update_live_filter();
        app.invalidate_caches();
        assert!(app.filter.live_filter_query.is_some());
        assert!(app.filter.live_filter_error.is_none());
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![2]);
    }

    #[test]
    fn test_live_filter_no_previous_keeps_none_on_error() {
        let mut app = App::new(100);
        // No previous valid query: error with live_filter_query still None
        app.filter.filter_input = Some(tui_input::Input::new("|".to_string()));
        app.filter.update_live_filter();
        assert!(app.filter.live_filter_query.is_none());
        assert!(app.filter.live_filter_error.is_some());
    }

    // --- command mode tests ---

    #[test]
    fn test_colon_enters_command_mode() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        assert!(app.command_input.is_none());

        app.handle_list_key(KeyCode::Char(':'), KeyModifiers::NONE, 10, 80);
        assert!(app.command_input.is_some());
    }

    #[test]
    fn test_command_esc_cancels() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        app.command_input = Some(tui_input::Input::new("record test".to_string()));
        app.command_error = None;

        app.handle_command_input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.command_input.is_none());
        assert!(app.command_error.is_none());
    }

    #[test]
    fn test_command_ctrl_c_cancels() {
        let mut app = App::new(100);
        app.add_line("hello".to_string());
        app.command_input = Some(tui_input::Input::new("record test".to_string()));

        app.handle_command_input(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.command_input.is_none());
    }

    #[test]
    fn test_command_ctrl_x_quits() {
        let mut app = App::new(100);
        app.command_input = Some(tui_input::Input::default());

        app.handle_command_input(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn test_command_unknown_shows_error() {
        let mut app = App::new(100);
        app.command_input = Some(tui_input::Input::new("foobar".to_string()));

        app.handle_command_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.command_input.is_none());
        assert!(app.command_error.is_some());
    }

    #[test]
    fn test_command_record_starts_recording() {
        let dir = std::env::temp_dir().join("logq_test_cmd_record");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cmd.log");

        let mut app = App::new(100);
        app.command_input = Some(tui_input::Input::new(format!("record {}", path.display())));

        app.handle_command_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.command_input.is_none());
        assert!(app.recorder.is_some());
        assert!(app.command_error.is_none());

        app.stop_recording();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_command_stoprecord_stops_recording() {
        let dir = std::env::temp_dir().join("logq_test_cmd_stoprecord");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cmd.log");

        let mut app = App::new(100);
        app.start_recording(path.clone()).unwrap();
        assert!(app.recorder.is_some());

        app.command_input = Some(tui_input::Input::new("stoprecord".to_string()));
        app.handle_command_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.recorder.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_command_record_no_path_shows_error() {
        let mut app = App::new(100);
        app.command_input = Some(tui_input::Input::new("record".to_string()));

        app.handle_command_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.command_error.is_some());
    }

    #[test]
    fn test_command_record_empty_path_shows_error() {
        let mut app = App::new(100);
        app.command_input = Some(tui_input::Input::new("record   ".to_string()));

        app.handle_command_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.command_error.is_some());
    }

    #[test]
    fn test_zz_centers_selection() {
        let mut app = App::new(100);
        for i in 0..50 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 25;
        app.scroll_offset = 0;
        app.auto_scroll = true;

        // First 'z' press: pending_z becomes true
        app.handle_list_key(KeyCode::Char('z'), KeyModifiers::NONE, 10, 67);
        assert!(app.pending_z);
        assert_eq!(app.selected, 25); // no movement yet
        assert_eq!(app.scroll_offset, 0); // no scroll change yet

        // Second 'z' press: centers the line
        app.handle_list_key(KeyCode::Char('z'), KeyModifiers::NONE, 10, 67);
        assert!(!app.pending_z);
        assert_eq!(app.selected, 25); // selection unchanged
        assert_eq!(app.scroll_offset, 20); // 25 - 10/2 = 20
        assert!(!app.auto_scroll);
    }

    #[test]
    fn test_pending_z_resets_on_other_key() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 5;

        app.handle_list_key(KeyCode::Char('z'), KeyModifiers::NONE, 10, 67);
        assert!(app.pending_z);

        app.handle_list_key(KeyCode::Char('j'), KeyModifiers::NONE, 10, 67);
        assert!(!app.pending_z);
        assert_eq!(app.selected, 6); // j moved normally
    }

    #[test]
    fn test_pending_z_resets_on_g() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 5;

        app.handle_list_key(KeyCode::Char('z'), KeyModifiers::NONE, 10, 67);
        assert!(app.pending_z);

        // 'g' should reset pending_z and set pending_g
        app.handle_list_key(KeyCode::Char('g'), KeyModifiers::NONE, 10, 67);
        assert!(!app.pending_z);
        assert!(app.pending_g);
    }

    #[test]
    fn test_single_z_no_action() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 5;
        app.scroll_offset = 0;

        app.handle_list_key(KeyCode::Char('z'), KeyModifiers::NONE, 10, 67);
        assert!(app.pending_z);
        assert_eq!(app.selected, 5);
        assert_eq!(app.scroll_offset, 0);
    }

    // --- context mode handler tests ---

    #[test]
    fn test_c_key_enters_context_mode() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        assert!(!app.context_mode);
        app.handle_list_key(KeyCode::Char('c'), KeyModifiers::NONE, 10, 67);
        assert!(app.context_mode);
    }

    #[test]
    fn test_esc_exits_context_mode() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        app.handle_list_key(KeyCode::Char('c'), KeyModifiers::NONE, 10, 67);
        assert!(app.context_mode);
        app.handle_list_key(KeyCode::Esc, KeyModifiers::NONE, 10, 67);
        assert!(!app.context_mode);
    }

    #[test]
    fn test_c_key_ignored_in_context_mode() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        app.handle_list_key(KeyCode::Char('c'), KeyModifiers::NONE, 10, 67);
        assert!(app.context_mode);
        let center = app.context_center;
        // Pressing c again should not re-enter (noop)
        app.handle_list_key(KeyCode::Char('c'), KeyModifiers::NONE, 10, 67);
        assert_eq!(app.context_center, center);
    }

    #[test]
    fn test_slash_ignored_in_context_mode() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        app.handle_list_key(KeyCode::Char('c'), KeyModifiers::NONE, 10, 67);
        app.handle_list_key(KeyCode::Char('/'), KeyModifiers::NONE, 10, 67);
        assert!(
            app.filter.filter_input.is_none(),
            "filter input should not activate in context mode"
        );
    }

    #[test]
    fn test_colon_ignored_in_context_mode() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        app.handle_list_key(KeyCode::Char('c'), KeyModifiers::NONE, 10, 67);
        app.handle_list_key(KeyCode::Char(':'), KeyModifiers::NONE, 10, 67);
        assert!(
            app.command_input.is_none(),
            "command input should not activate in context mode"
        );
    }

    #[test]
    fn test_esc_clears_filter_in_normal_mode() {
        use crate::filter::*;
        let mut app = App::new(100);
        app.add_line("aaa".to_string());
        app.add_line("bbb".to_string());
        app.filter.filter_query = Some(FilterQuery {
            segments: vec![FilterSegment::Plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("aaa".to_string()),
                regex: None,
                json_key: None,
            })],
        });
        app.invalidate_caches();
        assert!(app.filter.filter_query.is_some());
        app.handle_list_key(KeyCode::Esc, KeyModifiers::NONE, 10, 67);
        assert!(app.filter.filter_query.is_none());
    }

    #[test]
    fn test_navigation_works_in_context_mode() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        // selected starts at 0 (update_auto_scroll is called during rendering, not add_line)
        app.selected = 5;
        app.auto_scroll = false;

        app.handle_list_key(KeyCode::Char('c'), KeyModifiers::NONE, 10, 67);
        assert!(app.context_mode);
        assert_eq!(app.context_center, 5);

        app.handle_list_key(KeyCode::Char('k'), KeyModifiers::NONE, 10, 67);
        assert_eq!(app.selected, 4);
        app.handle_list_key(KeyCode::Char('j'), KeyModifiers::NONE, 10, 67);
        assert_eq!(app.selected, 5);
    }
}
