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

    t.press(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!t.app.should_quit);

    t.press(KeyCode::Char('c'), KeyModifiers::CONTROL);
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

    t.press(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!t.app.should_quit);

    t.press(KeyCode::Char('c'), KeyModifiers::CONTROL);
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
fn test_filter_backspace_empty_cancels() {
    let mut t = TestApp::new(10000);
    t.add_line("hello");
    t.render();

    // Enter filter mode
    t.press(KeyCode::Char('/'), KeyModifiers::NONE);
    assert!(t.app.filter_input.is_some());

    // Backspace on empty input should cancel filter input mode
    t.press(KeyCode::Backspace, KeyModifiers::NONE);
    assert!(t.app.filter_input.is_none());

    // Status bar should show normal list mode help, not filter input
    t.render();
    assert!(t.screen_contains("j/k:nav"));
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

    // Status bar should show filter syntax help
    assert!(t.screen_contains("|="));
    assert!(t.screen_contains("|~"));
    assert!(t.screen_contains("!="));
    assert!(t.screen_contains("!~"));
    assert!(t.screen_contains("Enter:apply"));
    assert!(t.screen_contains("Esc/Bksp:cancel"));
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
