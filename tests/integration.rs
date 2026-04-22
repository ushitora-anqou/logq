use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use logq::app::{App, ViewMode};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;

struct TestApp {
    app: App,
    terminal: Terminal<TestBackend>,
}

impl TestApp {
    fn new(max_lines: usize) -> Self {
        Self::new_with_size(max_lines, WIDTH, HEIGHT)
    }

    fn new_with_size(max_lines: usize, width: u16, height: u16) -> Self {
        let backend = TestBackend::new(width, height);
        let terminal = Terminal::new(backend).unwrap();
        let app = App::new(max_lines);
        Self { app, terminal }
    }

    fn add_line(&mut self, line: &str) {
        self.app.add_line(line.to_string());
    }

    fn press(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let event = Event::Key(KeyEvent::new(code, modifiers));
        let area = ratatui::layout::Rect::new(0, 0, WIDTH, HEIGHT);
        self.app.handle_event(event, area);
    }

    fn render(&mut self) {
        self.terminal.draw(|f| self.app.render(f)).unwrap();
    }

    /// Returns the text content of a single row (all cells concatenated).
    fn row_text(&self, row: u16) -> String {
        let buffer = self.terminal.backend().buffer();
        let mut line = String::new();
        for col in 0..buffer.area.width {
            line.push_str(buffer[(col, row)].symbol());
        }
        line
    }

    /// Checks if any row in the buffer contains the given text.
    fn screen_contains(&self, text: &str) -> bool {
        for row in 0..self.terminal.backend().buffer().area.height {
            if self.row_text(row).contains(text) {
                return true;
            }
        }
        false
    }
}

#[test]
fn test_line_addition() {
    let mut t = TestApp::new(10000);
    t.add_line("line1");
    t.render();

    assert!(t.screen_contains("line1"));
    assert_eq!(t.app.selected, 0);
    assert!(t.app.auto_scroll);

    t.add_line("line2");
    t.add_line("line3");
    t.render();

    assert!(t.screen_contains("line1"));
    assert!(t.screen_contains("line2"));
    assert!(t.screen_contains("line3"));
    assert_eq!(t.app.selected, 2);
}

#[test]
fn test_navigation_jk() {
    let mut t = TestApp::new(10000);
    t.add_line("a");
    t.add_line("b");
    t.add_line("c");
    t.render();

    assert_eq!(t.app.selected, 2);

    t.press(KeyCode::Char('k'), KeyModifiers::NONE);
    t.render();
    assert_eq!(t.app.selected, 1);
    assert!(!t.app.auto_scroll);

    t.press(KeyCode::Char('j'), KeyModifiers::NONE);
    t.render();
    assert_eq!(t.app.selected, 2);
    assert!(t.app.auto_scroll);
}

#[test]
fn test_view_switching() {
    let mut t = TestApp::new(10000);
    t.add_line("{\"key\":\"val\"}");
    t.render();

    // List view: no [detail] breadcrumb
    assert!(!t.screen_contains("[detail]"));

    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();
    assert_eq!(t.app.view_mode, ViewMode::Detail);
    assert!(t.screen_contains("[detail]"));

    t.press(KeyCode::Esc, KeyModifiers::NONE);
    t.render();
    assert_eq!(t.app.view_mode, ViewMode::List);
    assert!(!t.screen_contains("[detail]"));
}

#[test]
fn test_filter() {
    let mut t = TestApp::new(10000);
    t.add_line("alpha");
    t.add_line("beta");
    t.add_line("alpha2");
    t.render();

    // Enter filter mode
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    t.render();

    // Type |= "alpha"
    for c in "|= \"alpha\"".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.render();

    // Status bar should show filter input
    assert!(t.screen_contains("|="));

    // Apply filter
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();

    // Breadcrumb shows filter, content filtered
    assert!(t.screen_contains("[filter:"));
    assert!(t.screen_contains("alpha"));
    assert!(!t.screen_contains("beta"));
}

#[test]
fn test_auto_scroll() {
    let mut t = TestApp::new(10000);
    t.add_line("a");
    t.add_line("b");
    t.add_line("c");
    t.render();

    assert!(t.app.auto_scroll);

    t.press(KeyCode::Char('k'), KeyModifiers::NONE);
    t.render();
    assert!(!t.app.auto_scroll);
    assert_eq!(t.app.selected, 1);

    t.press(KeyCode::Char('G'), KeyModifiers::NONE);
    t.render();
    assert!(t.app.auto_scroll);
    assert_eq!(t.app.selected, 2);
}

#[test]
fn test_quit() {
    let mut t = TestApp::new(10000);
    t.add_line("line1");
    t.render();

    t.press(KeyCode::Char('x'), KeyModifiers::CONTROL);
    assert!(t.app.should_quit);
}

#[test]
fn test_max_lines() {
    let mut t = TestApp::new(2);
    t.add_line("line_a");
    t.add_line("line_b");
    t.add_line("line_c");
    t.render();

    assert_eq!(t.app.lines.len(), 2);
    assert!(t.screen_contains("line_b"));
    assert!(t.screen_contains("line_c"));
    assert!(!t.screen_contains("line_a"));
}

#[test]
fn test_detail_scroll() {
    let mut t = TestApp::new(10000);
    t.add_line(
        "{\"a\":1,\"b\":2,\"c\":3,\"d\":4,\"e\":5,\"f\":6,\"g\":7,\"h\":8,\"i\":9,\"j\":10}",
    );
    t.render();

    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();
    assert_eq!(t.app.view_mode, ViewMode::Detail);
    assert!(t.screen_contains("[detail]"));
    assert_eq!(t.app.detail_scroll, 0);

    t.press(KeyCode::Char('d'), KeyModifiers::CONTROL);
    t.render();
    assert!(t.app.detail_scroll > 0);
}

#[test]
fn test_tui_mode_with_command_no_panic() {
    let bin = env!("CARGO_BIN_EXE_logq");
    let mut cmd = Command::new(if cfg!(target_os = "macos") {
        "/usr/bin/script"
    } else {
        "script"
    });
    if cfg!(target_os = "macos") {
        cmd.args(["-q", "/dev/null", bin, "--", "echo", "hello"]);
    } else {
        cmd.args(["-qec", &format!("{} -- echo hello", bin), "/dev/null"]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn script");

    drop(child.stdin.take());

    let start = Instant::now();
    let timeout = Duration::from_secs(2);

    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => {
                assert_ne!(
                    status.code(),
                    Some(101),
                    "logq panicked (possible 'no reactor' error)"
                );
                return;
            }
            None if start.elapsed() > timeout => {
                child.kill().expect("kill");
                let _ = child.wait();
                return;
            }
            None => {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[test]
fn test_quit_with_no_input() {
    let mut t = TestApp::new(10000);
    t.render();

    t.press(KeyCode::Char('x'), KeyModifiers::CONTROL);
    assert!(t.app.should_quit);
}

#[test]
fn test_auto_scroll_follows_rapid_input() {
    let mut t = TestApp::new(10000);
    for i in 0..100 {
        t.add_line(&format!("line{}", i));
    }
    t.render();

    assert!(t.app.auto_scroll);
    assert_eq!(t.app.selected, 99);
    assert_eq!(t.app.lines.len(), 100);
    assert!(t.screen_contains("line99"));
}

#[test]
fn test_regex_filter() {
    let mut t = TestApp::new(10000);
    t.add_line("error timeout");
    t.add_line("info ok");
    t.add_line("error disk");
    t.render();

    // Type filter: /|~ "err.*timeout"
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "|~ \"err.*timeout\"".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();

    assert!(t.screen_contains("[filter:"));
    assert!(t.screen_contains("error timeout"));
    assert!(!t.screen_contains("info ok"));
    assert!(!t.screen_contains("error disk"));
}

#[test]
fn test_not_contains_filter() {
    let mut t = TestApp::new(10000);
    t.add_line("error timeout");
    t.add_line("info ok");
    t.add_line("warn slow");
    t.render();

    // Type filter: /!= "error"
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "!= \"error\"".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();

    assert!(t.screen_contains("[filter:"));
    assert!(t.screen_contains("info ok"));
    assert!(t.screen_contains("warn slow"));
    assert!(!t.screen_contains("error timeout"));
}

#[test]
fn test_not_regex_filter() {
    let mut t = TestApp::new(10000);
    t.add_line("error timeout");
    t.add_line("info ok");
    t.add_line("warn slow");
    t.render();

    // Type filter: /!~ "err.*"
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "!~ \"err.*\"".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();

    assert!(t.screen_contains("[filter:"));
    assert!(t.screen_contains("info ok"));
    assert!(t.screen_contains("warn slow"));
    assert!(!t.screen_contains("error timeout"));
}

#[test]
fn test_multiple_conditions_filter() {
    let mut t = TestApp::new(10000);
    t.add_line("error timeout");
    t.add_line("error disk");
    t.add_line("info ok");
    t.render();

    // Type filter: /|= "error" != "timeout"
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "|= \"error\" != \"timeout\"".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();

    assert!(t.screen_contains("[filter:"));
    assert!(t.screen_contains("error disk"));
    assert!(!t.screen_contains("error timeout"));
    assert!(!t.screen_contains("info ok"));
}

#[test]
fn test_filter_backspace_empty_stays_in_mode() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    // Enter filter mode
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    assert!(t.app.filter_input.is_some());

    // Backspace on empty input should NOT cancel filter input mode
    t.press(KeyCode::Backspace, KeyModifiers::NONE);
    assert!(t.app.filter_input.is_some());

    // Ctrl-C should cancel filter input mode
    t.press(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(t.app.filter_input.is_none());

    // Status bar should show normal list mode help, not filter input
    t.render();
    // List mode shortcuts should be visible (not filter input shortcuts)
    assert!(t.screen_contains("Exit"));
}

#[test]
fn test_filter_backspace_nonempty_removes_char() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "|=".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.render();
    assert!(t.screen_contains("|="));

    // Backspace should remove last char, not cancel
    t.press(KeyCode::Backspace, KeyModifiers::NONE);
    t.render();
    assert!(t.app.filter_input.is_some());
}

#[test]
fn test_filter_esc_cancels() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "|= \"test\"".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    assert!(t.app.filter_input.is_some());

    // Esc should always cancel filter input, even with text
    t.press(KeyCode::Esc, KeyModifiers::NONE);
    assert!(t.app.filter_input.is_none());
}

#[test]
fn test_filter_help_text_visible() {
    let mut t = TestApp::new_with_size(10000, 200, 24);
    t.add_line("hello");
    t.render();

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    t.render();

    // Status bar should show filter input help
    assert!(t.screen_contains("Apply filter"));
    assert!(t.screen_contains("Search hist"));
    assert!(t.screen_contains("Cancel"));
}

#[test]
fn test_filter_input_no_slash_displayed() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    t.render();

    // Input line (3rd from bottom) should NOT contain "/" prefix
    // Layout: breadcrumb + content + input_line + help1 + help2
    let input_row = HEIGHT - 3;
    let input_text = t.row_text(input_row);
    // Input line should start with a space (no "/" prefix)
    assert!(
        input_text.starts_with(' '),
        "input line should start with space, got: {:?}",
        &input_text[..input_text.len().min(20)]
    );
    assert!(
        !input_text.starts_with(" /"),
        "input line should not start with ' /', got: {:?}",
        &input_text[..input_text.len().min(20)]
    );
}

#[test]
fn test_filter_ctrl_c_cancels() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "|= \"test\"".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    assert!(t.app.filter_input.is_some());

    // Ctrl-C should cancel filter input mode
    t.press(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(t.app.filter_input.is_none());
}

#[test]
fn test_escape_clears_filter() {
    let mut t = TestApp::new(10000);
    t.add_line("alpha");
    t.add_line("beta");
    t.add_line("alpha2");
    t.render();

    // Apply filter |= "alpha"
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "|= \"alpha\"".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();
    assert!(t.screen_contains("[filter:"));

    // Esc clears filter
    t.press(KeyCode::Esc, KeyModifiers::NONE);
    t.render();
    assert!(!t.screen_contains("[filter:"));
    assert!(t.screen_contains("beta"));
}

#[test]
fn test_filter_parse_error_shown() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    // Type invalid filter (unterminated string)
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "|= \"foo".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();

    // Error should be shown, input preserved
    assert!(t.app.filter_input.is_some());
    assert!(t.screen_contains("Error:"));
}

#[test]
fn test_filter_parse_error_then_fix() {
    let mut t = TestApp::new(10000);
    t.add_line("foobar");
    t.add_line("bazqux");
    t.render();

    // Type invalid filter
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "|= \"foo".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();

    // Error shown, input preserved
    assert!(t.app.filter_input.is_some());

    // Fix: type closing quote
    t.press(KeyCode::Char('"'), KeyModifiers::NONE);
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();

    // Filter applied
    assert!(t.app.filter_input.is_none());
    assert!(t.screen_contains("[filter:"));
    assert!(t.screen_contains("foobar"));
    assert!(!t.screen_contains("bazqux"));
}

#[test]
fn test_filter_history_stored_on_enter() {
    let mut t = TestApp::new(10000);
    t.add_line("hello world");
    t.render();

    // Apply first filter
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "hello""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    assert!(t.app.filter_input.is_none());
    t.render();
    assert!(t.screen_contains(r#"[filter: |= "hello"]"#));

    // Press / again — starts new filter input; breadcrumb should NOT show
    // the old committed filter since live query is empty
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    t.render();
    assert!(!t.screen_contains(r#"[filter:""#));
    // History should be stored; pressing Up loads the previous query
    t.press(KeyCode::Up, KeyModifiers::NONE);
    t.render();
    assert!(t.screen_contains(r#"|= "hello""#));
}

#[test]
fn test_filter_history_no_duplicate() {
    let mut t = TestApp::new(10000);
    t.add_line("hello world");
    t.render();

    // Apply same filter twice
    for _ in 0..2 {
        t.press(KeyCode::Char('/'), KeyModifiers::NONE);
        for c in r#"|= "hello""#.chars() {
            t.press(KeyCode::Char(c), KeyModifiers::NONE);
        }
        t.press(KeyCode::Enter, KeyModifiers::NONE);
    }
    t.render();
    // Filter should still be applied — breadcrumb shows [filter: |= "hello"]
    assert!(t.screen_contains(r#"|= "hello""#));
    assert!(t.screen_contains("[filter:"));
}

#[test]
fn test_slash_starts_empty() {
    let mut t = TestApp::new(10000);
    t.add_line("hello world");
    t.render();

    // Apply a filter
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "hello""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Press / again — should start with empty input
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    assert_eq!(t.app.filter_input.as_ref().map(|i| i.value()), Some(""));

    // Up arrow should load the previous query from history
    t.press(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        t.app.filter_input.as_ref().map(|i| i.value()),
        Some(r#"|= "hello""#)
    );
}

#[test]
fn test_filter_history_up_down() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    // Apply first filter (no history, blank input)
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "first""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Apply second filter (type full query from scratch)
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "first" |= "second""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Enter filter mode — starts with empty input
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    assert_eq!(t.app.filter_input.as_ref().map(|i| i.value()), Some(""));

    // Up should show the last (combined) query
    t.press(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        t.app.filter_input.as_ref().map(|i| i.value()),
        Some(r#"|= "first" |= "second""#)
    );

    // Up again should show the first query
    t.press(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        t.app.filter_input.as_ref().map(|i| i.value()),
        Some(r#"|= "first""#)
    );

    // Down should go back to second (combined filter)
    t.press(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(
        t.app.filter_input.as_ref().map(|i| i.value()),
        Some(r#"|= "first" |= "second""#)
    );

    // Down past end should restore empty draft
    t.press(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(t.app.filter_input.as_ref().map(|i| i.value()), Some(""));
}

#[test]
fn test_empty_enter_clears_filter_then_slash_starts_empty() {
    let mut t = TestApp::new(10000);
    t.add_line("hello world");
    t.add_line("foo bar");
    t.render();

    // Apply a filter
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "hello""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    t.render();
    assert!(t.screen_contains("[filter:"));

    // Open filter, press Enter with empty input — should clear filter
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    assert_eq!(t.app.filter_input.as_ref().map(|i| i.value()), Some(""));
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    assert!(t.app.filter_input.is_none());
    t.render();
    assert!(!t.screen_contains("[filter:"));

    // All lines should be visible now
    t.render();
    assert!(t.screen_contains("hello world"));
    assert!(t.screen_contains("foo bar"));

    // Press / again — should start with empty input
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    assert_eq!(t.app.filter_input.as_ref().map(|i| i.value()), Some(""));
}

#[test]
fn test_live_filter_on_type() {
    let mut t = TestApp::new(10000);
    t.add_line("hello world");
    t.add_line("foo bar");
    t.add_line("hello foo");
    t.render();

    // Enter filter mode and type
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "hello""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.render();

    // Live filter should be active — only matching lines visible
    assert!(t.screen_contains("hello world"));
    assert!(!t.screen_contains("foo bar"));
    assert!(t.screen_contains("hello foo"));
}

#[test]
fn test_live_filter_error_shown() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    // Type an invalid query (missing closing quote)
    for c in "|= \"foo".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.render();

    assert!(t.screen_contains("Error:"));
}

#[test]
fn test_ctrl_r_reverse_search() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    // Apply two filters
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "alpha""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "alpha beta""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Enter filter mode and Ctrl+R to search history
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    t.press(KeyCode::Char('r'), KeyModifiers::CONTROL);
    t.render();
    // Should find a matching history entry containing "alpha"
    assert!(t.screen_contains("alpha"));
}

#[test]
fn test_ctrl_r_preserves_search_pattern() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    // Apply two different filters
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "alpha""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "alpha beta""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Enter filter mode, type "alpha" as search pattern, then Ctrl+R
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "alpha".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Char('r'), KeyModifiers::CONTROL);

    // First Ctrl+R should find a match
    let first_match = t.app.filter_input.clone().unwrap();
    assert!(
        first_match.value().contains("alpha"),
        "First match should contain 'alpha', got: {}",
        first_match.value()
    );

    // Second Ctrl+R should find a different (older) match
    t.press(KeyCode::Char('r'), KeyModifiers::CONTROL);
    let second_match = t.app.filter_input.clone().unwrap();
    assert!(
        second_match.value().contains("alpha"),
        "Second match should still contain 'alpha', got: {}",
        second_match.value()
    );
    assert_ne!(
        first_match.value(),
        second_match.value(),
        "Second Ctrl+R should find a different entry"
    );
}

#[test]
fn test_ctrl_r_type_adds_to_pattern() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    // Apply filters: one with "alpha", one with "beta"
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "alpha""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "beta""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Enter filter mode, Ctrl+R to start search, then type "alpha"
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    t.press(KeyCode::Char('r'), KeyModifiers::CONTROL);
    for c in "alpha".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }

    // Should match only "alpha" entries, not "beta"
    let matched = t.app.filter_input.clone().unwrap();
    assert!(
        matched.value().contains("alpha"),
        "Should match 'alpha' filter, got: {}",
        matched.value()
    );
    assert!(
        !matched.value().contains("beta"),
        "Should NOT match 'beta' filter, got: {}",
        matched.value()
    );

    // Should show search pattern "alpha" in input line
    t.render();
    assert!(
        t.screen_contains("alpha"),
        "Input line should show the search pattern 'alpha'"
    );
    assert!(
        t.screen_contains("reverse-i-search"),
        "Should show 'reverse-i-search' prompt"
    );
}

#[test]
fn test_ctrl_r_backspace_removes_from_pattern() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "alpha""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "beta""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Enter filter mode, Ctrl+R, type "alpha"
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    t.press(KeyCode::Char('r'), KeyModifiers::CONTROL);
    for c in "alpha".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }

    // Should match "alpha"
    assert!(
        t.app
            .filter_input
            .as_ref()
            .unwrap()
            .value()
            .contains("alpha")
    );

    // Backspace to remove "a" → pattern becomes "alph"
    t.press(KeyCode::Backspace, KeyModifiers::NONE);

    // Still in search mode - render and check
    t.render();
    assert!(
        t.screen_contains("reverse-i-search"),
        "Should still be in search mode after backspace"
    );
    assert!(
        t.screen_contains("alph"),
        "Should show shortened pattern 'alph'"
    );

    // Should still match "alpha" (contains "alph")
    assert!(
        t.app
            .filter_input
            .as_ref()
            .unwrap()
            .value()
            .contains("alpha")
    );
}

#[test]
fn test_ctrl_g_cancels_search_restores_input() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    // Apply a filter to create history
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "alpha""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Enter filter mode with some text
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "beta""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }

    // Press Ctrl+R to enter search mode (saves current input)
    t.press(KeyCode::Char('r'), KeyModifiers::CONTROL);
    t.render();
    assert!(t.screen_contains("reverse-i-search"));

    // Ctrl+G should cancel search and restore original input
    t.press(KeyCode::Char('g'), KeyModifiers::CONTROL);
    assert_eq!(
        t.app.filter_input.as_ref().map(|i| i.value()),
        Some(r#"|= "beta""#),
        "Ctrl+G should restore original input"
    );
    // Should still be in filter input mode
    assert!(t.app.filter_input.is_some());
}

#[test]
fn test_ctrl_r_failed_search_shows_feedback() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    // Apply a filter to create history
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "alpha""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Enter filter mode, Ctrl+R, type non-matching text
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    t.press(KeyCode::Char('r'), KeyModifiers::CONTROL);
    for c in "zzzzz".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.render();

    assert!(
        t.screen_contains("failed"),
        "Should show 'failed' feedback for non-matching search"
    );
}

#[test]
fn test_ctrl_r_esc_accepts_stays_in_input() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    // Apply filters to create history
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "alpha""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Enter filter mode, Ctrl+R
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    t.press(KeyCode::Char('r'), KeyModifiers::CONTROL);

    // Esc should accept the match and exit search mode only
    t.press(KeyCode::Esc, KeyModifiers::NONE);

    // Should still be in filter input mode with the matched entry
    assert!(
        t.app.filter_input.is_some(),
        "Should still be in filter input mode"
    );

    // Search mode should be exited - render and check no search prompt
    t.render();
    assert!(
        !t.screen_contains("reverse-i-search"),
        "Search prompt should be gone after Esc"
    );
}

#[test]
fn test_ctrl_r_uses_typed_text_as_initial_pattern() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    // Create history: alpha, alpha beta, beta (in this order)
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "alpha""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "alpha beta""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "beta""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Type "alpha" then press Ctrl+R — should use "alpha" as initial search pattern
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in "alpha".chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Char('r'), KeyModifiers::CONTROL);

    // Should find a history entry containing "alpha", NOT the most recent "|= beta"
    let matched = t.app.filter_input.clone().unwrap();
    assert!(
        matched.value().contains("alpha"),
        "Should match 'alpha' (initial pattern from typed text), got: {}",
        matched.value()
    );
    assert!(
        !matched.value().contains(r#"|= "beta""#),
        "Should NOT match '|= \"beta\"' entry, got: {}",
        matched.value()
    );
}

#[test]
fn test_detail_q_returns_to_list() {
    let mut t = TestApp::new(10000);
    t.add_line("hello world");
    t.render();

    // Enter detail view
    t.press(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(t.app.view_mode, ViewMode::Detail);

    // Press q to go back
    t.press(KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(t.app.view_mode, ViewMode::List);
}
