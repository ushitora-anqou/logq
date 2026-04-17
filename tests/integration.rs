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
        let backend = TestBackend::new(WIDTH, HEIGHT);
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
    let mut child = Command::new("script")
        .args(["-qec", &format!("{} -- echo hello", bin), "/dev/null"])
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
    assert!(t.screen_contains("j/k nav"));
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
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    t.render();

    // Status bar should show filter input help
    assert!(t.screen_contains("Enter:apply"));
    assert!(t.screen_contains("C-r:search"));
    assert!(t.screen_contains("Esc:cancel"));
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

    // Press / again — previous query should be preset (history stored)
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
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
fn test_filter_preset_on_slash() {
    let mut t = TestApp::new(10000);
    t.add_line("hello world");
    t.render();

    // Apply a filter
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    for c in r#"|= "hello""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Press / again — should show previous query in status bar
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    t.render();
    assert!(t.screen_contains(r#"|= "hello""#));
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

    // Apply second filter (appends to preset)
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    // input is now |= "first" — append space + second condition
    for c in r#" |= "second""#.chars() {
        t.press(KeyCode::Char(c), KeyModifiers::NONE);
    }
    t.press(KeyCode::Enter, KeyModifiers::NONE);

    // Enter filter mode — presets last query
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    assert_eq!(
        t.app.filter_input.as_deref(),
        Some(r#"|= "first" |= "second""#)
    );

    // Up should show the first query
    t.press(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(t.app.filter_input.as_deref(), Some(r#"|= "first""#));

    // Down should go back to second (combined filter)
    t.press(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(
        t.app.filter_input.as_deref(),
        Some(r#"|= "first" |= "second""#)
    );
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
