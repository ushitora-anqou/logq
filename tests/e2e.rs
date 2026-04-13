use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(serde::Deserialize)]
struct AppState {
    lines: Vec<String>,
    #[allow(dead_code)]
    timestamps: Vec<String>,
    view_mode: String,
    selected: usize,
    #[allow(dead_code)]
    scroll_offset: usize,
    filter: Option<String>,
    auto_scroll: bool,
    detail_scroll: u16,
    filter_input: Option<String>,
    should_quit: bool,
    total_lines: usize,
    filtered_count: usize,
}

fn run_logq(input: &str) -> Vec<AppState> {
    run_logq_with_args(&[], input)
}

fn run_logq_with_args(args: &[&str], input: &str) -> Vec<AppState> {
    let bin = env!("CARGO_BIN_EXE_logq");
    let mut child = Command::new(bin)
        .args(args)
        .arg("--json-output")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn logq");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        write!(stdin, "{}", input).expect("write stdin");
    }

    let output = child.wait_with_output().expect("wait");
    assert!(output.status.success(), "logq exited with error");

    output
        .stdout
        .lines()
        .map(|line| serde_json::from_str(&line.expect("utf8")).expect("parse json"))
        .collect()
}

#[test]
fn test_line_addition() {
    let states = run_logq("line1\nline2\nline3\n");
    assert_eq!(states.len(), 3);

    assert_eq!(states[0].lines, vec!["line1"]);
    assert_eq!(states[0].selected, 0);
    assert!(states[0].auto_scroll);

    assert_eq!(states[2].lines, vec!["line1", "line2", "line3"]);
    assert_eq!(states[2].selected, 2);
    assert!(states[2].auto_scroll);
}

#[test]
fn test_navigation_jk() {
    let states = run_logq("a\nb\nc\nCMD:k\nCMD:j\n");
    // states: [a, b, c, after-CMD:k, after-CMD:j]
    assert_eq!(states.len(), 5);

    // After adding c: selected=2 (auto_scroll)
    assert_eq!(states[2].selected, 2);
    // CMD:k -> selected=1
    assert_eq!(states[3].selected, 1);
    assert!(!states[3].auto_scroll);
    // CMD:j -> selected=2
    assert_eq!(states[4].selected, 2);
    assert!(states[4].auto_scroll);
}

#[test]
fn test_view_switching() {
    let states = run_logq("{\"key\":\"val\"}\nCMD:Enter\nCMD:Esc\n");
    assert_eq!(states.len(), 3);

    assert_eq!(states[0].view_mode, "list");
    assert_eq!(states[1].view_mode, "detail");
    assert_eq!(states[1].detail_scroll, 0);
    assert_eq!(states[2].view_mode, "list");
}

#[test]
fn test_filter() {
    let states =
        run_logq("alpha\nbeta\nalpha2\nCMD:/\nCMD:a\nCMD:l\nCMD:p\nCMD:h\nCMD:a\nCMD:Enter\n");
    assert_eq!(states.len(), 10);

    // After typing filter input
    assert_eq!(states[4].filter_input.as_deref(), Some("a"));
    assert_eq!(states[5].filter_input.as_deref(), Some("al"));
    assert_eq!(states[8].filter_input.as_deref(), Some("alpha"));

    // After Enter: filter applied
    assert_eq!(states[9].filter.as_deref(), Some("alpha"));
    assert_eq!(states[9].filtered_count, 2); // "alpha" and "alpha2"
    assert_eq!(states[9].filter_input, None);
}

#[test]
fn test_auto_scroll() {
    let states = run_logq("a\nb\nc\nCMD:k\nCMD:G\n");
    assert_eq!(states.len(), 5);

    // After c: auto_scroll=true
    assert!(states[2].auto_scroll);
    // CMD:k: auto_scroll=false
    assert!(!states[3].auto_scroll);
    assert_eq!(states[3].selected, 1);
    // CMD:G: auto_scroll=true, selected=last
    assert!(states[4].auto_scroll);
    assert_eq!(states[4].selected, 2);
}

#[test]
fn test_quit() {
    let states = run_logq("line1\nCMD:C-c\nCMD:C-c\n");
    assert_eq!(states.len(), 3);

    // First C-c: no quit
    assert!(!states[1].should_quit);
    // Second C-c: quit
    assert!(states[2].should_quit);
}

#[test]
fn test_max_lines() {
    let states = run_logq_with_args(&["--max-lines", "2"], "a\nb\nc\n");
    assert_eq!(states.len(), 3);

    assert_eq!(states[0].lines, vec!["a"]);
    assert_eq!(states[1].lines, vec!["a", "b"]);
    assert_eq!(states[2].lines, vec!["b", "c"]);
    assert_eq!(states[2].total_lines, 2);
}

#[test]
fn test_detail_scroll() {
    let states = run_logq("{\"a\":1,\"b\":2,\"c\":3}\nCMD:Enter\nCMD:C-d\n");
    assert_eq!(states.len(), 3);

    assert_eq!(states[1].view_mode, "detail");
    assert_eq!(states[1].detail_scroll, 0);
    // C-d scrolls by half visible height (12)
    assert!(states[2].detail_scroll > 0);
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
    let states = run_logq("CMD:C-c\nCMD:C-c\n");
    assert_eq!(states.len(), 2);
    assert!(!states[0].should_quit);
    assert!(states[1].should_quit);
}

#[test]
fn test_auto_scroll_follows_rapid_input() {
    let mut input = String::new();
    for i in 0..100 {
        input.push_str(&format!("line{}\n", i));
    }
    let states = run_logq(&input);
    assert_eq!(states.len(), 100);
    let last = &states[99];
    assert!(last.auto_scroll);
    assert_eq!(last.selected, 99);
    assert_eq!(last.total_lines, 100);
}

#[test]
fn test_timestamp_display() {
    let states = run_logq("hello\n");
    assert_eq!(states.len(), 1);
    // Timestamp format: HH:MM:SS.mmm (12 chars)
    let ts = &states[0].timestamps[0];
    assert_eq!(ts.len(), 12);
    assert_eq!(&ts[2..3], ":");
    assert_eq!(&ts[5..6], ":");
    assert_eq!(&ts[8..9], ".");
}

#[test]
fn test_regex_filter() {
    let states = run_logq(
        "error timeout\ninfo ok\nerror disk\nCMD:/\nCMD:e\nCMD:r\nCMD:r\nCMD:.\nCMD:*\nCMD:t\nCMD:i\nCMD:m\nCMD:e\nCMD:o\nCMD:u\nCMD:t\nCMD:Enter\n",
    );
    // Find the state after Enter
    let final_state = states.last().unwrap();
    assert_eq!(final_state.filter.as_deref(), Some("err.*timeout"));
    assert_eq!(final_state.filtered_count, 1);
}

#[test]
fn test_not_filter() {
    let states = run_logq(
        "error timeout\ninfo ok\nwarn slow\nCMD:/\nCMD:!\nCMD:e\nCMD:r\nCMD:r\nCMD:o\nCMD:r\nCMD:Enter\n",
    );
    let final_state = states.last().unwrap();
    assert_eq!(final_state.filter.as_deref(), Some("error"));
    assert_eq!(final_state.filtered_count, 2); // "info ok" and "warn slow"
}

#[test]
fn test_escape_clears_filter() {
    let states = run_logq(
        "alpha\nbeta\nalpha2\nCMD:/\nCMD:a\nCMD:l\nCMD:p\nCMD:h\nCMD:a\nCMD:Enter\nCMD:Esc\n",
    );
    // After Enter (index 9): filter is set
    let after_enter = &states[9];
    assert_eq!(after_enter.filter.as_deref(), Some("alpha"));
    // After Esc (index 10): filter is cleared
    let after_esc = &states[10];
    assert_eq!(after_esc.filter, None);
    assert_eq!(after_esc.filtered_count, 3);
}
