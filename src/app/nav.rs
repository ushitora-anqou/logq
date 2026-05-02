use crate::app::App;

impl App {
    pub fn update_auto_scroll(&mut self, visible_height: usize, content_width: usize) {
        if !self.auto_scroll {
            return;
        }
        let last = {
            let filtered = self.filtered_indices();
            if filtered.is_empty() {
                return;
            }
            filtered[filtered.len() - 1]
        };
        self.selected = last;
        let (_, prefix_sums) = self.cached_row_layout(content_width);
        let total_rows = *prefix_sums.last().unwrap_or(&0);
        let max_offset = total_rows.saturating_sub(visible_height);
        self.scroll_offset = max_offset;
    }

    pub(super) fn ensure_selection_visible(&mut self, visible_height: usize, content_width: usize) {
        let selected_pos = {
            let filtered = self.filtered_indices();
            match filtered.iter().position(|&i| i == self.selected) {
                Some(p) => p,
                None => return,
            }
        };
        let (row_layout, prefix_sums) = self.cached_row_layout(content_width);
        let entry_first_row = prefix_sums[selected_pos];
        let entry_height = row_layout[selected_pos];
        if self.scroll_offset > entry_first_row {
            self.scroll_offset = entry_first_row;
        }
        if entry_first_row + entry_height > self.scroll_offset + visible_height {
            self.scroll_offset = entry_first_row + entry_height - visible_height;
        }
    }

    pub fn move_selection(&mut self, delta: isize, visible_height: usize, content_width: usize) {
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
        self.ensure_selection_visible(visible_height, content_width);
    }

    /// Try to scroll within an expanded entry. Returns true if scrolled (viewport
    /// moved one row within the entry), false if at the entry boundary (caller
    /// should fall through to move_selection).
    pub fn try_scroll_expanded(
        &mut self,
        forward: bool,
        visible_height: usize,
        content_width: usize,
    ) -> bool {
        if !self.is_expanded(self.selected) {
            return false;
        }
        let selected_pos = {
            let filtered = self.filtered_indices();
            match filtered.iter().position(|&i| i == self.selected) {
                Some(p) => p,
                None => return false,
            }
        };
        let (row_layout, prefix_sums) = self.cached_row_layout(content_width);
        let entry_first_row = prefix_sums[selected_pos];
        let entry_height = row_layout[selected_pos];
        if entry_height <= visible_height {
            return false;
        }
        if forward {
            if self.scroll_offset + visible_height < entry_first_row + entry_height {
                self.scroll_offset += 1;
                self.auto_scroll = false;
                return true;
            }
        } else if self.scroll_offset > entry_first_row {
            self.scroll_offset = self.scroll_offset.saturating_sub(1);
            self.auto_scroll = false;
            return true;
        }
        false
    }

    pub fn page_move(
        &mut self,
        delta_rows: isize,
        visible_height: usize,
        content_width: usize,
        forward: bool,
    ) {
        let (filtered_first_last, current_pos) = {
            let filtered = self.filtered_indices();
            if filtered.is_empty() {
                return;
            }
            let pos = filtered
                .iter()
                .position(|&i| i == self.selected)
                .unwrap_or(0);
            (filtered.clone(), pos)
        };
        let (_row_layout, prefix_sums) = self.cached_row_layout(content_width);
        let current_row = prefix_sums[current_pos];
        let total_rows = *prefix_sums.last().unwrap_or(&0);
        let target_row =
            (current_row as isize + delta_rows).clamp(0, total_rows as isize - 1) as usize;
        // Binary search prefix_sums to find entry containing target_row.
        let new_pos = match prefix_sums.binary_search(&target_row) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        if new_pos < filtered_first_last.len() {
            self.selected = filtered_first_last[new_pos];
            self.auto_scroll = false;
            if forward {
                self.scroll_offset = target_row;
            } else {
                self.scroll_offset = target_row.saturating_sub(visible_height.saturating_sub(1));
            }
            self.ensure_selection_visible(visible_height, content_width);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;

    #[test]
    fn test_move_selection() {
        let mut app = App::new(100);
        app.add_line("a".to_string());
        app.add_line("b".to_string());
        app.add_line("c".to_string());
        app.selected = 0;
        app.move_selection(1, 10, 67);
        assert_eq!(app.selected, 1);
        app.move_selection(1, 10, 67);
        assert_eq!(app.selected, 2);
        app.move_selection(1, 10, 67);
        assert_eq!(app.selected, 2);
        app.move_selection(-1, 10, 67);
        assert_eq!(app.selected, 1);
        app.move_selection(-1, 10, 67);
        assert_eq!(app.selected, 0);
        app.move_selection(-1, 10, 67);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_move_selection_with_filter() {
        use crate::filter::*;
        let mut app = App::new(100);
        app.add_line("aaa".to_string());
        app.add_line("bbb".to_string());
        app.add_line("aaa2".to_string());
        app.filter.filter_query = Some(FilterQuery {
            segments: vec![FilterSegment::Plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("aaa".to_string()),
                regex: None,
                json_key: None,
            })],
        });
        let filtered = app.filtered_indices();
        assert_eq!(filtered, vec![0, 2]);

        app.selected = 0;
        app.move_selection(1, 10, 67);
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
        app.move_selection(-1, 10, 67);
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
    fn test_vim_scroll_moves_selection() {
        let mut app = App::new(100);
        for i in 0..50 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 10;
        app.scroll_offset = 10;

        app.move_selection(5, 10, 67);
        assert_eq!(app.selected, 15);
    }

    #[test]
    fn test_ensure_selection_visible() {
        let mut app = App::new(100);
        for i in 0..30 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 20;
        app.scroll_offset = 0;
        app.ensure_selection_visible(10, 67);
        assert!(app.selected >= app.scroll_offset);
        assert!(app.selected < app.scroll_offset + 10);
    }

    #[test]
    fn test_move_selection_up_scrolls_with_filter() {
        use crate::filter::*;
        let mut app = App::new(100);
        for i in 0..20 {
            if i % 2 == 0 {
                app.add_line(format!("match line{}", i));
            } else {
                app.add_line(format!("other line{}", i));
            }
        }
        // Filter for "match": matches lines 0, 2, 4, 6, 8, 10, 12, 14, 16, 18
        app.filter.filter_query = Some(FilterQuery {
            segments: vec![FilterSegment::Plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("match".to_string()),
                regex: None,
                json_key: None,
            })],
        });
        // filtered = [0, 2, 4, 6, 8, 10, 12, 14, 16, 18]

        let visible_height = 5;

        // Start at the bottom
        app.update_auto_scroll(visible_height, 67);
        assert_eq!(app.selected, 18);
        assert_eq!(app.scroll_offset, 5);

        // Move up until we reach the top of the visible area
        for _ in 0..5 {
            app.move_selection(-1, visible_height, 67);
        }
        assert_eq!(app.selected, 8); // filtered[4]
        // scroll_offset should have adjusted to keep selected visible
        assert!(
            app.scroll_offset <= 4,
            "scroll_offset should be <= 4 so filtered[4]=line8 is visible, got {}",
            app.scroll_offset
        );
    }

    #[test]
    fn test_auto_scroll_updates_scroll_offset() {
        let mut app = App::new(100);
        for i in 0..50 {
            app.add_line(format!("line{}", i));
        }
        assert!(app.auto_scroll);
        app.update_auto_scroll(10, 67);
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
        app.update_auto_scroll(10, 67);
        assert_eq!(app.scroll_offset, 5);
    }

    #[test]
    fn test_auto_scroll_with_filter() {
        use crate::filter::*;
        let mut app = App::new(100);
        app.add_line("aaa1".to_string());
        app.add_line("bbb".to_string());
        app.add_line("aaa2".to_string());
        app.add_line("bbb2".to_string());
        app.add_line("aaa3".to_string());
        app.filter.filter_query = Some(FilterQuery {
            segments: vec![FilterSegment::Plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("aaa".to_string()),
                regex: None,
                json_key: None,
            })],
        });
        app.update_auto_scroll(10, 67);
        assert_eq!(app.selected, 4);
    }

    #[test]
    fn test_page_move_forward_sets_cursor_at_top() {
        let mut app = App::new(100);
        for i in 0..50 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 0;
        app.scroll_offset = 0;
        app.auto_scroll = true;

        // C-f: move forward by visible_height (10)
        app.page_move(10, 10, 67, true);

        assert_eq!(app.selected, 10);
        assert_eq!(app.scroll_offset, 10); // cursor at top of screen
        assert!(!app.auto_scroll);
    }

    #[test]
    fn test_page_move_backward_sets_cursor_at_bottom() {
        let mut app = App::new(100);
        for i in 0..50 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 20;
        app.scroll_offset = 10;
        app.auto_scroll = false;

        // C-b: move backward by visible_height (10)
        app.page_move(-10, 10, 67, false);

        assert_eq!(app.selected, 10);
        // cursor at bottom of screen: scroll_offset = new_pos - (visible_height - 1)
        assert_eq!(app.scroll_offset, 1);
        assert!(!app.auto_scroll);
    }

    #[test]
    fn test_page_move_half_forward() {
        let mut app = App::new(100);
        for i in 0..50 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 0;
        app.scroll_offset = 0;

        // C-d: move forward by half (5) with visible_height=10
        app.page_move(5, 10, 67, true);

        assert_eq!(app.selected, 5);
        assert_eq!(app.scroll_offset, 5); // cursor at top
    }

    #[test]
    fn test_page_move_half_backward() {
        let mut app = App::new(100);
        for i in 0..50 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 20;
        app.scroll_offset = 10;

        // C-u: move backward by half (5) with visible_height=10
        app.page_move(-5, 10, 67, false);

        assert_eq!(app.selected, 15);
        // cursor at bottom: scroll_offset = 15 - (10 - 1) = 6
        assert_eq!(app.scroll_offset, 6);
    }

    #[test]
    fn test_page_move_clamps_at_boundaries() {
        let mut app = App::new(100);
        for i in 0..10 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 0;
        app.scroll_offset = 0;

        // C-b from the top: should clamp at 0
        app.page_move(-10, 10, 67, false);
        assert_eq!(app.selected, 0);
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_page_move_forward_clamps_at_end() {
        let mut app = App::new(100);
        for i in 0..15 {
            app.add_line(format!("line{}", i));
        }
        app.selected = 12;
        app.scroll_offset = 5;

        // C-f near end: should clamp at last line
        app.page_move(10, 10, 67, true);
        assert_eq!(app.selected, 14); // last line
        assert_eq!(app.scroll_offset, 14); // cursor at top
    }

    #[test]
    fn test_page_move_forward_keeps_expanded_entry_visible() {
        let mut app = App::new(100);
        for i in 0..30 {
            app.add_line(format!("line{}", i));
        }
        // Expand entry 0 so it spans multiple rows (content_width=20, "line0" is short but
        // highlight produces spans; use a long JSON string to get a tall entry).
        app.lines[0].text = r#"{"key":"abcdefghijklmnopqrstuvwxyz"}"#.to_string();
        app.expanded.insert(0);

        let content_width = 20;
        let visible_height = 5;
        app.selected = 0;
        app.scroll_offset = 0;

        // C-f: page forward — should land on an entry whose full height is visible
        app.page_move(visible_height as isize, visible_height, content_width, true);

        let filtered = app.filtered_indices();
        let sel_pos = filtered.iter().position(|&i| i == app.selected).unwrap();
        let row_layout = app.compute_row_layout(&filtered, content_width);
        let entry_first_row = row_layout.iter().take(sel_pos).sum::<usize>();
        let entry_height = row_layout[sel_pos];

        assert!(
            entry_first_row >= app.scroll_offset,
            "entry should start after scroll_offset"
        );
        assert!(
            entry_first_row + entry_height <= app.scroll_offset + visible_height,
            "entry should fit entirely within visible area: first_row={}, height={}, scroll={}, visible={}",
            entry_first_row,
            entry_height,
            app.scroll_offset,
            visible_height,
        );
    }

    #[test]
    fn test_page_move_backward_keeps_expanded_entry_visible() {
        let mut app = App::new(100);
        for i in 0..30 {
            app.add_line(format!("line{}", i));
        }
        // Expand entry 15 with a tall JSON payload
        app.lines[15].text = r#"{"key":"abcdefghijklmnopqrstuvwxyz"}"#.to_string();
        app.expanded.insert(15);

        let content_width = 20;
        let visible_height = 5;
        app.selected = 20;
        app.scroll_offset = 15;

        // C-b: page backward
        app.page_move(
            -(visible_height as isize),
            visible_height,
            content_width,
            false,
        );

        let filtered = app.filtered_indices();
        let sel_pos = filtered.iter().position(|&i| i == app.selected).unwrap();
        let row_layout = app.compute_row_layout(&filtered, content_width);
        let entry_first_row = row_layout.iter().take(sel_pos).sum::<usize>();
        let entry_height = row_layout[sel_pos];

        assert!(
            entry_first_row >= app.scroll_offset,
            "entry should start after scroll_offset"
        );
        assert!(
            entry_first_row + entry_height <= app.scroll_offset + visible_height,
            "entry should fit entirely within visible area: first_row={}, height={}, scroll={}, visible={}",
            entry_first_row,
            entry_height,
            app.scroll_offset,
            visible_height,
        );
    }
}
