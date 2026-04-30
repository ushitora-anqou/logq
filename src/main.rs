use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::time::Duration;

use clap::Parser;
use logq::app::App;
use logq::input::InputLine;

/// logq - TUI viewer for NDJSON and text streams with live tailing, regex filtering, and vim keybindings
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Maximum number of lines to keep in memory
    #[arg(long, default_value = "10000")]
    max_lines: usize,

    /// Read from a file instead of stdin or a command
    #[arg(long = "file")]
    file: Option<String>,

    /// Command to execute. Use `logq -- command args` when the command starts with `-`
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

/// When stdin is a pipe, dup it to a new fd and replace fd 0 with /dev/tty.
/// This allows crossterm to read keyboard events from /dev/tty while we
/// read data from the original stdin via the returned File.
fn redirect_stdin_to_tty() -> io::Result<Option<File>> {
    if unsafe { libc::isatty(0) } == 1 {
        return Ok(None);
    }

    // Save original stdin to a new fd
    let saved_fd = unsafe { libc::dup(0) };
    if saved_fd == -1 {
        return Err(io::Error::last_os_error());
    }
    let saved_stdin = unsafe { File::from_raw_fd(saved_fd) };

    // Open /dev/tty and replace fd 0
    let tty = File::open("/dev/tty").map_err(|e| {
        io::Error::new(
            e.kind(),
            "failed to open /dev/tty: logq requires a terminal when reading from a pipe",
        )
    })?;
    if unsafe { libc::dup2(tty.as_raw_fd(), 0) } == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(Some(saved_stdin))
}

fn detect_and_set_locale() {
    let locale = sys_locale::get_locale().unwrap_or_else(|| "en".to_string());
    let matched = if locale.starts_with("zh_CN") || locale.starts_with("zh-Hans") {
        "zh-CN"
    } else {
        match locale.split(['_', '-']).next().unwrap_or("en") {
            "ja" => "ja",
            _ => "en",
        }
    };
    rust_i18n::set_locale(matched);
}

fn main() -> io::Result<()> {
    detect_and_set_locale();

    // Ignore SIGPIPE so logq never dies from writing to a closed pipe
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let cli = Cli::parse();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let _guard = rt.enter();

    let command = if cli.command.is_empty() {
        None
    } else {
        Some(cli.command)
    };

    if cli.file.is_some() && command.is_some() {
        eprintln!("error: --file and command arguments are mutually exclusive");
        std::process::exit(1);
    }

    let saved_stdin = redirect_stdin_to_tty()?;
    let is_pipe_mode = saved_stdin.is_some() && command.is_none() && cli.file.is_none();

    let mut terminal = ratatui::init();
    let logq::input::InputReader {
        rx,
        child_pid,
        task_handle,
    } = if let Some(file_path) = &cli.file {
        let file = File::open(file_path).unwrap_or_else(|e| {
            eprintln!("error: cannot open file '{}': {}", file_path, e);
            std::process::exit(1);
        });
        logq::input::spawn_line_reader(None, Some(file))
    } else if command.is_none() && saved_stdin.is_none() {
        // No input source (TTY without pipe) — skip line reader to avoid fd conflict with crossterm
        let (_, rx) = tokio::sync::mpsc::unbounded_channel();
        logq::input::InputReader {
            rx,
            child_pid: None,
            task_handle: None,
        }
    } else {
        logq::input::spawn_line_reader(command, saved_stdin)
    };

    let mut app = App::new(cli.max_lines);
    app.load_history();
    let result = run_app(&mut terminal, &mut app, rx);

    cleanup_input_reader(child_pid, task_handle, &rt);

    // Save state and restore terminal before any signal-based cleanup
    app.save_history();
    ratatui::restore();

    if is_pipe_mode {
        terminate_pipeline_upstream();
    }

    result
}

/// Terminate the spawned child process and await the reader task.
/// On command mode the entire process group is signalled (SIGTERM, then
/// SIGKILL after a 1s grace period); on stdin/pipe mode the reader task
/// is simply aborted.
fn cleanup_input_reader(
    child_pid: Option<u32>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
    rt: &tokio::runtime::Runtime,
) {
    if let Some(pid) = child_pid {
        let pgid = pid as libc::pid_t;
        unsafe { libc::kill(-pgid, libc::SIGTERM) };
        rt.block_on(async {
            if let Some(handle) = task_handle {
                tokio::select! {
                    _ = handle => {}
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        unsafe { libc::kill(-pgid, libc::SIGKILL) };
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        });
    } else if let Some(handle) = task_handle {
        handle.abort();
    }
}

/// Pipe mode: terminate the upstream command in the pipeline.
/// After the reader task aborts the pipe is closed and the upstream should
/// receive SIGPIPE. As a fallback, also send SIGTERM to the process group
/// (handles commands that ignore SIGPIPE or haven't written yet).
fn terminate_pipeline_upstream() {
    let pgid = unsafe { libc::getpgrp() };
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
        libc::kill(-pgid, libc::SIGTERM);
    }
    std::thread::sleep(Duration::from_millis(100));
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_DFL);
    }
}

fn run_app(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<InputLine>,
) -> io::Result<()> {
    // Initial render
    terminal.draw(|frame| app.render(frame))?;

    loop {
        let mut needs_render = false;

        // Receive new lines (non-blocking)
        while let Ok(line) = rx.try_recv() {
            app.add_line_with_source(line.text, line.source);
            needs_render = true;
        }

        if app.should_quit {
            return Ok(());
        }

        // Poll for events with a short timeout
        if app.poll_events()? {
            let event = app.next_event()?;
            let area = terminal.get_frame().area();
            app.handle_event(event, area);
            needs_render = true;
        }

        if needs_render {
            terminal.draw(|frame| app.render(frame))?;
        }
    }
}
