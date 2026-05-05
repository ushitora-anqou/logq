use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::app::{App, ShortcutItem, TIMESTAMP_WIDTH};
use crate::highlight::highlight_line;
use crate::render::*;

impl App {
    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let (row1, row2, num_cols, key_widths) = self.shortcut_items();

        let input_mode = self.filter.filter_input.is_some() || self.command_input.is_some();

        if input_mode {
            // Input mode: titlebar + content + input + status + shortcuts
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(area);

            self.render_titlebar(frame, chunks[0]);
            self.render_list(frame, chunks[1]);
            self.render_input_line(frame, chunks[2]);
            self.render_status_line(frame, chunks[3]);
            self.render_shortcut_bar(frame, chunks[4], &row1, num_cols, &key_widths);
            self.render_shortcut_bar(frame, chunks[5], &row2, num_cols, &key_widths);

            let cursor_x = if let Some(pattern) = &self.filter.history_search_pattern {
                let label = if self.filter.history_search_failed {
                    t!("input.failed_reverse_i_search").to_string()
                } else {
                    t!("input.reverse_i_search").to_string()
                };
                let prefix_len = display_width(&label) + 1; // +1 for the "'" suffix
                (1 + prefix_len + display_width(pattern)) as u16
            } else if let Some(input) = &self.command_input {
                (2 + input.visual_cursor()) as u16 // +2 for ": " prefix
            } else {
                let input = self.filter.filter_input.as_ref().unwrap();
                (1 + input.visual_cursor()) as u16
            };
            frame.set_cursor_position((cursor_x, chunks[2].y));
        } else {
            // Normal mode: titlebar + content + status + shortcuts
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

            self.render_titlebar(frame, chunks[0]);
            self.render_list(frame, chunks[1]);
            self.render_status_line(frame, chunks[2]);
            self.render_shortcut_bar(frame, chunks[3], &row1, num_cols, &key_widths);
            self.render_shortcut_bar(frame, chunks[4], &row2, num_cols, &key_widths);
        }

        if self.show_help {
            self.render_help(frame, area);
        }
    }

    fn render_titlebar(&self, frame: &mut Frame, area: Rect) {
        let width = area.width as usize;
        let reversed = Style::default()
            .fg(Color::White)
            .bg(Color::Black)
            .add_modifier(Modifier::REVERSED);

        // Build center text
        let mut center_parts = Vec::new();
        if let Some(q) = self.active_filter_query() {
            center_parts.push(t!("titlebar.filter_prefix", query = q.display_string()).to_string());
        }
        if let Some(recorder) = &self.recorder {
            let path = recorder.path().display().to_string();
            center_parts.push(t!("titlebar.recording", path = path).to_string());
        }
        let center_text = center_parts.join(" > ");

        // Layout: "logq" on left, centered status, padding to fill width
        let left = "logq";
        let left_len = display_width(left);
        let center_len = display_width(&center_text);

        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::styled(left.to_string(), reversed));

        // Calculate padding to center the status text
        let remaining = width.saturating_sub(left_len).saturating_sub(center_len);
        let pad_left = remaining / 2;
        let pad_right = remaining - pad_left;

        if !center_text.is_empty() {
            spans.push(Span::styled(" ".repeat(pad_left), reversed));
            spans.push(Span::styled(center_text, reversed));
            spans.push(Span::styled(" ".repeat(pad_right), reversed));
        } else {
            spans.push(Span::styled(
                " ".repeat(width.saturating_sub(left_len)),
                reversed,
            ));
        }

        // Ensure we fill the width exactly
        let total_len: usize = spans.iter().map(|s| display_width(&s.content)).sum();
        if total_len < width {
            spans.push(Span::styled(
                " ".repeat(width.saturating_sub(total_len)),
                reversed,
            ));
        }

        let paragraph = Paragraph::new(Line::from(spans));
        frame.render_widget(paragraph, area);
    }

    pub(crate) fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let filtered = self.filtered_indices();
        let width = area.width as usize;
        let visible_height = area.height as usize;
        let content_width = width.saturating_sub(TIMESTAMP_WIDTH);

        // auto_scroll: follow the latest line
        self.update_auto_scroll(visible_height, content_width);

        // Compute row layout (cached) and clamp scroll_offset
        let (row_layout, prefix_sums) = self.cached_row_layout(content_width);
        let row_layout = row_layout.to_vec();
        let prefix_sums = prefix_sums.to_vec();
        let total_rows = *prefix_sums.last().unwrap_or(&0);
        let max_offset = total_rows.saturating_sub(visible_height);
        self.scroll_offset = self.scroll_offset.min(max_offset);

        // Find the first entry that overlaps with the visible area using binary
        // search on the prefix sums; this avoids an O(N) linear scan when the
        // viewport is far from index 0 (e.g. auto-scrolled to the bottom).
        let start_pos = match prefix_sums.binary_search(&self.scroll_offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let mut display_rows: Vec<Line<'static>> = Vec::new();
        let mut accumulated_rows = prefix_sums.get(start_pos).copied().unwrap_or(0);
        let mut rows_remaining = visible_height;

        for (pos, &idx) in filtered.iter().enumerate().skip(start_pos) {
            if rows_remaining == 0 {
                break;
            }
            let entry_height = row_layout[pos];
            let entry_first_row = accumulated_rows;
            accumulated_rows += entry_height;

            // Skip entries entirely above the viewport
            if accumulated_rows <= self.scroll_offset {
                continue;
            }
            // Skip entries entirely below the viewport
            if entry_first_row >= self.scroll_offset + visible_height {
                break;
            }

            // Calculate how many rows of this entry to skip (if it starts above viewport)
            let skip_rows = self.scroll_offset.saturating_sub(entry_first_row);

            let entry = &self.lines[idx];
            let is_selected = idx == self.selected;
            let is_expanded = self.is_expanded(idx);

            // Source prefix
            let (prefix_str, prefix_width, prefix_style) = entry.source_prefix();

            // Timestamp span
            let ts_span = Span::styled(
                format!("{} ", entry.timestamp),
                Style::default().fg(Color::DarkGray),
            );
            let ts_pad = Span::raw(" ".repeat(TIMESTAMP_WIDTH));

            // Source prefix span (if any)
            let prefix_span: Vec<Span<'static>> = if prefix_width > 0 {
                vec![Span::styled(prefix_str.to_string(), prefix_style)]
            } else {
                vec![]
            };

            if is_expanded {
                let display_text = self.display_text_for(idx);
                let highlighted = highlight_line(&display_text, &self.colors);
                let mut row_idx = 0usize;
                for (i, hl_line) in highlighted.lines.into_iter().enumerate() {
                    let wrapped = wrap_line(&hl_line, content_width);
                    if wrapped.is_empty() {
                        if row_idx >= skip_rows && rows_remaining > 0 {
                            let mut spans: Vec<Span<'static>> = Vec::new();
                            if i == 0 {
                                spans.push(ts_span.clone());
                                spans.extend(prefix_span.iter().cloned());
                            } else {
                                spans.push(ts_pad.clone());
                            }
                            if is_selected {
                                spans.push(Span::styled(
                                    " ".repeat(content_width),
                                    Style::default().bg(Color::DarkGray),
                                ));
                            }
                            display_rows.push(Line::from(spans));
                            rows_remaining -= 1;
                        }
                        row_idx += 1;
                        continue;
                    }
                    for (j, wrapped_part) in wrapped.into_iter().enumerate() {
                        if row_idx >= skip_rows && rows_remaining > 0 {
                            let mut spans: Vec<Span<'static>> = Vec::new();
                            if i == 0 && j == 0 {
                                spans.push(ts_span.clone());
                                spans.extend(prefix_span.iter().cloned());
                            } else {
                                spans.push(ts_pad.clone());
                            }
                            if is_selected {
                                spans.extend(apply_selected_style(wrapped_part));
                            } else {
                                spans.extend(wrapped_part);
                            }
                            display_rows.push(Line::from(spans));
                            rows_remaining -= 1;
                        }
                        row_idx += 1;
                    }
                }
            } else {
                // Collapsed: single row
                if skip_rows == 0 && rows_remaining > 0 {
                    let display = truncate_str(
                        &self.display_text_for(idx),
                        content_width.saturating_sub(prefix_width),
                    );
                    let content_spans = highlight_display_line(&display, &self.colors, is_selected);
                    let mut spans: Vec<Span<'static>> = vec![ts_span];
                    spans.extend(prefix_span);
                    if is_selected {
                        spans.extend(apply_selected_style(content_spans));
                    } else {
                        spans.extend(content_spans);
                    }
                    display_rows.push(Line::from(spans));
                    rows_remaining -= 1;
                }
            }
        }

        let text = Text::from(display_rows);
        let paragraph = Paragraph::new(text);
        frame.render_widget(paragraph, area);
    }

    fn render_input_line(&self, frame: &mut Frame, area: Rect) {
        let bg = Style::default().bg(Color::DarkGray);
        let width = area.width as usize;

        // Command input mode
        if let Some(input) = &self.command_input {
            let mut s: Vec<Span<'static>> = vec![Span::styled(
                format!(":{}", input.value()),
                Style::default().fg(Color::White).bg(Color::DarkGray),
            )];
            let left_len: usize = s.iter().map(|sp| display_width(&sp.content)).sum();
            let padding = width.saturating_sub(left_len);
            if padding > 0 {
                s.push(Span::styled(" ".repeat(padding), bg));
            }
            let status = Paragraph::new(Line::from(s));
            frame.render_widget(status, area);
            return;
        }

        // Filter input mode
        let mut s: Vec<Span<'static>> = if let Some(pattern) = &self.filter.history_search_pattern {
            let label = if self.filter.history_search_failed {
                t!("input.failed_reverse_i_search").to_string()
            } else {
                t!("input.reverse_i_search").to_string()
            };
            let matched = self
                .filter
                .filter_input
                .as_ref()
                .map(|i| i.value())
                .unwrap_or("");
            vec![
                Span::styled(
                    format!(" {}'{}': ", label, pattern),
                    Style::default().fg(Color::Yellow).bg(Color::DarkGray),
                ),
                Span::styled(
                    matched.to_string(),
                    Style::default().fg(Color::White).bg(Color::DarkGray),
                ),
            ]
        } else {
            let input = self
                .filter
                .filter_input
                .as_ref()
                .map(|i| i.value())
                .unwrap_or("");
            vec![Span::styled(
                format!(" {}", input),
                Style::default().fg(Color::White).bg(Color::DarkGray),
            )]
        };

        let left_len: usize = s.iter().map(|sp| display_width(&sp.content)).sum();
        let padding = width.saturating_sub(left_len);
        if padding > 0 {
            s.push(Span::styled(" ".repeat(padding), bg));
        }

        let status = Paragraph::new(Line::from(s));
        frame.render_widget(status, area);
    }

    fn render_status_line(&mut self, frame: &mut Frame, area: Rect) {
        let width = area.width as usize;

        // Command error takes priority
        if let Some(err) = &self.command_error {
            let err_prefix = t!("status.error_prefix").to_string();
            let full_len = display_width(&err_prefix) + display_width(err) + 1;
            let spans = vec![
                Span::styled(
                    format!("{}{}", err_prefix, err),
                    Style::default().fg(Color::Red),
                ),
                Span::raw(" ".repeat(width.saturating_sub(full_len))),
            ];
            let paragraph = Paragraph::new(Line::from(spans));
            frame.render_widget(paragraph, area);
            return;
        }

        let error = self
            .filter
            .live_filter_error
            .as_deref()
            .or(self.filter.filter_error.as_deref());

        if let Some(err) = error {
            let err_prefix = t!("status.error_prefix").to_string();
            let full_len = display_width(&err_prefix) + display_width(err) + 1; // +1 for leading space
            let spans = vec![
                Span::styled(
                    format!("{}{}", err_prefix, err),
                    Style::default().fg(Color::Red),
                ),
                Span::raw(" ".repeat(width.saturating_sub(full_len))),
            ];
            let paragraph = Paragraph::new(Line::from(spans));
            frame.render_widget(paragraph, area);
            return;
        }

        let mut parts = Vec::new();

        let filtered = self.filtered_indices();
        if let Some(pos) = filtered.iter().position(|&i| i == self.selected) {
            parts.push(format!("{}/{}", pos + 1, filtered.len()));
        }
        if self.filter.filter_query.is_some() || self.filter.live_filter_query.is_some() {
            parts.push(t!("status.total", count = self.lines.len()).to_string());
        }

        if self.process_exited {
            parts.push(t!("status.exited").to_string());
        }

        if let Some(recorder) = &self.recorder {
            let path = recorder.path().display().to_string();
            parts.push(t!("status.recording", path = path).to_string());
        }

        if self.auto_scroll {
            parts.push(t!("status.follow").to_string());
        } else {
            parts.push(t!("status.scroll").to_string());
        }

        let status_text = parts.join(" │ ");
        let text_len = display_width(&status_text) + 1;
        let spans = vec![
            Span::styled(
                format!(" {}", status_text),
                Style::default().fg(Color::Gray),
            ),
            Span::raw(" ".repeat(width.saturating_sub(text_len))),
        ];
        let paragraph = Paragraph::new(Line::from(spans));
        frame.render_widget(paragraph, area);
    }

    fn render_shortcut_bar(
        &self,
        frame: &mut Frame,
        area: Rect,
        items: &[ShortcutItem],
        num_cols: usize,
        key_widths: &[usize; 8],
    ) {
        let width = area.width as usize;

        if num_cols == 0 {
            let paragraph = Paragraph::new(Line::from(Span::raw(" ".repeat(width))));
            frame.render_widget(paragraph, area);
            return;
        }

        let col_width = width / num_cols;
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut col_idx = 0;

        for (i, item) in items.iter().enumerate() {
            if item.key.is_empty() {
                // Fill remaining columns with spaces
                let remaining_cols = num_cols.saturating_sub(col_idx);
                if remaining_cols > 0 {
                    spans.push(Span::raw(" ".repeat(col_width * remaining_cols)));
                    col_idx = num_cols;
                }
                break;
            }

            let kw = key_widths[i].min(col_width.saturating_sub(2));
            let key_padded = format!("{:width$}", item.key, width = kw);
            // Reserve: key + 1 space + at least 1 padding space
            let desc_available = col_width.saturating_sub(kw + 2);
            let desc_text = if desc_available > 0 {
                truncate_str(&item.desc, desc_available)
            } else {
                String::new()
            };
            let padding = col_width
                .saturating_sub(kw + 1)
                .saturating_sub(display_width(&desc_text));

            // Key part: reverse video
            spans.push(Span::styled(
                key_padded,
                Style::default().add_modifier(Modifier::REVERSED),
            ));
            // Space + description + padding
            spans.push(Span::raw(format!(
                "{}{}{}",
                " ",
                desc_text,
                " ".repeat(padding)
            )));
            col_idx += 1;
        }

        // Fill remaining columns with spaces (if loop didn't cover all)
        let remaining_cols = num_cols.saturating_sub(col_idx);
        if remaining_cols > 0 {
            spans.push(Span::raw(" ".repeat(col_width * remaining_cols)));
        }

        // Ensure we fill the entire width
        let total_len: usize = spans.iter().map(|s| display_width(&s.content)).sum();
        if total_len < width {
            spans.push(Span::raw(" ".repeat(width - total_len)));
        }

        let paragraph = Paragraph::new(Line::from(spans));
        frame.render_widget(paragraph, area);
    }

    fn render_help(&mut self, frame: &mut Frame, area: Rect) {
        let key_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let section_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(Color::DarkGray);

        let entries: Vec<(String, Vec<(&str, String)>)> = vec![
            (
                t!("help.section.navigation").to_string(),
                vec![
                    ("j / Down", t!("help.key.move_down").to_string()),
                    ("k / Up", t!("help.key.move_up").to_string()),
                    ("G", t!("help.key.jump_to_end").to_string()),
                    ("gg", t!("help.key.jump_to_top").to_string()),
                    ("^D", t!("help.key.half_page_down").to_string()),
                    ("^U", t!("help.key.half_page_up").to_string()),
                    ("^F", t!("help.key.full_page_down").to_string()),
                    ("^B", t!("help.key.full_page_up").to_string()),
                    ("^E", t!("help.key.one_line_down").to_string()),
                    ("^Y", t!("help.key.one_line_up").to_string()),
                ],
            ),
            (
                t!("help.section.view").to_string(),
                vec![
                    ("Enter", t!("help.key.toggle_expand").to_string()),
                    ("^O", t!("help.key.expand_collapse_all").to_string()),
                    ("y", t!("help.key.yank_copy_line").to_string()),
                ],
            ),
            (
                t!("help.section.filter").to_string(),
                vec![
                    ("/", t!("help.key.start_filter_input").to_string()),
                    ("Esc", t!("help.key.clear_filter").to_string()),
                    ("", "Query: |= \"str\" |~ /re/".to_string()),
                    ("", "      | key = \"val\"".to_string()),
                    ("", "      | line_format \"{{ .k }}\"".to_string()),
                ],
            ),
            (
                t!("help.section.other").to_string(),
                vec![
                    (":", t!("help.key.command").to_string()),
                    ("^G", t!("help.key.help").to_string()),
                    ("^X", t!("help.key.exit_logq").to_string()),
                ],
            ),
        ];

        let mut lines: Vec<Line<'static>> = Vec::new();

        for (section, bindings) in &entries {
            lines.push(Line::from(vec![Span::styled(
                section.clone(),
                section_style,
            )]));
            for (key, desc) in bindings {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<12}", key), key_style),
                    Span::raw(desc.clone()),
                ]));
            }
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![Span::styled(
            t!("help.press_esc_to_close").to_string(),
            dim,
        )]));

        let content_height = lines.len() as u16;
        let content_width = lines
            .iter()
            .map(|line| line.width() as u16)
            .max()
            .unwrap_or(40)
            .max(40)
            .min(area.width.saturating_sub(4));

        let popup_width = (content_width + 4).min(area.width);
        let popup_height = (content_height + 2).min(area.height);

        let help_area = Rect {
            x: area.width.saturating_sub(popup_width) / 2,
            y: area.height.saturating_sub(popup_height) / 2,
            width: popup_width,
            height: popup_height,
        };

        frame.render_widget(Clear, help_area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Span::styled(t!("help.title").to_string(), key_style));
        let inner = block.inner(help_area);
        frame.render_widget(block, help_area);

        let visible_height = inner.height;
        let max_scroll = content_height.saturating_sub(visible_height);
        self.help_scroll = self.help_scroll.min(max_scroll);

        frame.render_widget(
            Paragraph::new(Text::from(lines)).scroll((self.help_scroll, 0)),
            inner,
        );
    }

    fn shortcut_items(&self) -> ([ShortcutItem; 8], [ShortcutItem; 8], usize, [usize; 8]) {
        let (row1, row2, num_cols) = if self.command_input.is_some() {
            (
                [
                    ShortcutItem {
                        key: "Enter",
                        desc: t!("shortcut.execute").to_string(),
                    },
                    ShortcutItem {
                        key: "Esc",
                        desc: t!("shortcut.cancel").to_string(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                ],
                [
                    ShortcutItem {
                        key: "^C",
                        desc: t!("shortcut.cancel").to_string(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                    ShortcutItem {
                        key: "",
                        desc: String::new(),
                    },
                ],
                8,
            )
        } else if self.filter.filter_input.is_some() {
            if self.filter.history_search_pattern.is_some() {
                (
                    [
                        ShortcutItem {
                            key: "^R",
                            desc: t!("shortcut.next_match").to_string(),
                        },
                        ShortcutItem {
                            key: "^G",
                            desc: t!("shortcut.cancel_search").to_string(),
                        },
                        ShortcutItem {
                            key: "Enter",
                            desc: t!("shortcut.apply_filter").to_string(),
                        },
                        ShortcutItem {
                            key: "Esc",
                            desc: t!("shortcut.accept_match").to_string(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                    ],
                    [
                        ShortcutItem {
                            key: "Bksp",
                            desc: t!("shortcut.delete_char").to_string(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                    ],
                    8,
                )
            } else {
                (
                    [
                        ShortcutItem {
                            key: "Enter",
                            desc: t!("shortcut.apply_filter").to_string(),
                        },
                        ShortcutItem {
                            key: "Up/Dn",
                            desc: t!("shortcut.history").to_string(),
                        },
                        ShortcutItem {
                            key: "^R",
                            desc: t!("shortcut.search_hist").to_string(),
                        },
                        ShortcutItem {
                            key: "Esc",
                            desc: t!("shortcut.cancel").to_string(),
                        },
                        ShortcutItem {
                            key: "^G",
                            desc: t!("shortcut.cancel_search").to_string(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                    ],
                    [
                        ShortcutItem {
                            key: "Bksp",
                            desc: t!("shortcut.delete_char").to_string(),
                        },
                        ShortcutItem {
                            key: "^C",
                            desc: t!("shortcut.cancel").to_string(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                        ShortcutItem {
                            key: "",
                            desc: String::new(),
                        },
                    ],
                    8,
                )
            }
        } else {
            (
                [
                    ShortcutItem {
                        key: "j",
                        desc: t!("help.key.move_down").to_string(),
                    },
                    ShortcutItem {
                        key: "k",
                        desc: t!("help.key.move_up").to_string(),
                    },
                    ShortcutItem {
                        key: "^E",
                        desc: t!("shortcut.one_line_down").to_string(),
                    },
                    ShortcutItem {
                        key: "^Y",
                        desc: t!("shortcut.one_line_up").to_string(),
                    },
                    ShortcutItem {
                        key: "^D",
                        desc: t!("shortcut.half_pg_down").to_string(),
                    },
                    ShortcutItem {
                        key: "^U",
                        desc: t!("shortcut.half_pg_up").to_string(),
                    },
                    ShortcutItem {
                        key: "^B",
                        desc: t!("shortcut.full_pg_up").to_string(),
                    },
                    ShortcutItem {
                        key: "^F",
                        desc: t!("shortcut.full_pg_down").to_string(),
                    },
                ],
                [
                    ShortcutItem {
                        key: "gg",
                        desc: t!("help.key.jump_to_top").to_string(),
                    },
                    ShortcutItem {
                        key: "G",
                        desc: t!("help.key.jump_to_end").to_string(),
                    },
                    ShortcutItem {
                        key: "Enter",
                        desc: t!("help.key.toggle_expand").to_string(),
                    },
                    ShortcutItem {
                        key: "^O",
                        desc: t!("shortcut.expand_all").to_string(),
                    },
                    ShortcutItem {
                        key: "/",
                        desc: t!("shortcut.filter_lines").to_string(),
                    },
                    ShortcutItem {
                        key: ":",
                        desc: t!("help.key.command").to_string(),
                    },
                    ShortcutItem {
                        key: "^X",
                        desc: t!("help.key.exit_logq").to_string(),
                    },
                    ShortcutItem {
                        key: "^G",
                        desc: t!("help.key.help").to_string(),
                    },
                ],
                8,
            )
        };

        // Compute per-column key widths: max key length between row1 and row2 for each column
        let key_widths: [usize; 8] =
            std::array::from_fn(|i| row1[i].key.len().max(row2[i].key.len()));

        (row1, row2, num_cols, key_widths)
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::filter::*;
    use crate::input::LineSource;

    fn plain(condition: FilterCondition) -> FilterSegment {
        FilterSegment::Plain(condition)
    }

    fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push(
                    buf.cell((x, y))
                        .unwrap()
                        .symbol()
                        .chars()
                        .next()
                        .unwrap_or(' '),
                );
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn test_breadcrumb_updates_live_during_filter_input() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = App::new(100);
        app.add_line("foo line".to_string());
        app.add_line("bar line".to_string());

        // Set a committed filter
        app.filter.filter_query = Some(FilterQuery {
            segments: vec![plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String("foo".to_string()),
                regex: None,
                json_key: None,
            })],
        });
        app.cache.filtered_indices = None;

        // Start typing a new filter query
        app.filter.filter_input = Some(tui_input::Input::new(r#"|= "bar""#.to_string()));
        app.filter.update_live_filter();

        // Render and check breadcrumb shows the live filter
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let rendered = buffer_to_string(&buf);

        assert!(
            rendered.contains(r#"|= "bar""#),
            "breadcrumb should show live filter query during input, got: {}",
            rendered
        );
        assert!(
            !rendered.contains(r#"|= "foo""#),
            "breadcrumb should NOT show old committed filter during input, got: {}",
            rendered
        );
    }

    #[test]
    fn test_stderr_line_renders_with_prefix() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = App::new(100);
        app.add_line_with_source("stdout line".to_string(), LineSource::Stdout);
        app.add_line_with_source("stderr line".to_string(), LineSource::Stderr);

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let rendered = buffer_to_string(&buf);

        assert!(
            rendered.contains("[stderr] stderr line"),
            "stderr line should have [stderr] prefix, got: {}",
            rendered
        );
        assert!(
            !rendered.contains("[stderr] stdout"),
            "stdout line should not have [stderr] prefix, got: {}",
            rendered
        );
    }

    #[test]
    fn test_system_line_renders_with_prefix() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = App::new(100);
        app.add_line_with_source("process exited with code 0".to_string(), LineSource::System);

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let rendered = buffer_to_string(&buf);

        assert!(
            rendered.contains("[logq] process exited with code 0"),
            "system line should have [logq] prefix, got: {}",
            rendered
        );
    }
}
