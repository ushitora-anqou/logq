use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::time::Duration;

use clap::Parser;
use logq::app::App;

/// logq - TUI viewer for NDJSON and text streams with live tailing, regex filtering, and vim keybindings
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Maximum number of lines to keep in memory
    #[arg(long, default_value = "10000")]
    max_lines: usize,

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

fn main() -> io::Result<()> {
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

    let saved_stdin = redirect_stdin_to_tty()?;
    let is_pipe_mode = saved_stdin.is_some() && command.is_none();

    let mut terminal = ratatui::init();
    let (rx, mut child, reader_handle) = if command.is_none() && saved_stdin.is_none() {
        // No input source (TTY without pipe) — skip line reader to avoid fd conflict with crossterm
        let (_, rx) = tokio::sync::mpsc::unbounded_channel();
        (rx, None, None)
    } else {
        logq::input::spawn_line_reader(command, saved_stdin)
    };

    let mut app = App::new(cli.max_lines);
    app.load_history();
    let result = run_app(&mut terminal, &mut app, rx, &mut child);

    // Command mode: kill spawned child process group
    if let Some(ref mut c) = child
        && let Some(pid) = c.id()
    {
        let pgid = pid as libc::pid_t;
        // Send SIGTERM to the entire process group
        unsafe { libc::kill(-pgid, libc::SIGTERM) };
        rt.block_on(async {
            tokio::select! {
                _ = c.wait() => {}
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    // Send SIGKILL to the entire process group
                    unsafe { libc::kill(-pgid, libc::SIGKILL) };
                    let _ = c.start_kill();
                    let _ = c.wait().await;
                }
            }
        });
    }

    // Save state and restore terminal before any signal-based cleanup
    app.save_history();
    ratatui::restore();

    // Abort the reader task to close the pipe read end promptly.
    // Without this, the async task holds the file handle indefinitely,
    // preventing the upstream command from receiving SIGPIPE.
    if let Some(handle) = reader_handle {
        handle.abort();
    }

    // Pipe mode: terminate the upstream command in the pipeline.
    // After abort(), the pipe read end is closed and the upstream should
    // get SIGPIPE. As a fallback, also send SIGTERM to the process group
    // (handles commands that ignore SIGPIPE or haven't written yet).
    if is_pipe_mode {
        let pgid = unsafe { libc::getpgrp() };
        // Protect ourselves from the SIGTERM we're about to send
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
            libc::kill(-pgid, libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(100));
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
        }
    }

    result
}

fn run_app(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    _child: &mut Option<tokio::process::Child>,
) -> io::Result<()> {
    // Initial render
    terminal.draw(|frame| app.render(frame))?;

    loop {
        let mut needs_render = false;

        // Receive new lines (non-blocking)
        while let Ok(line) = rx.try_recv() {
            app.add_line(line);
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
