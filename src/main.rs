mod app;
mod highlight;
mod input;

use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd};

use app::App;
use clap::Parser;

/// logq - NDJSON TUI viewer with live tailing and vim-style keybindings
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Maximum number of lines to keep in memory
    #[arg(long, default_value = "10000")]
    max_lines: usize,

    /// Command to execute (prefix with --)
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
    let cli = Cli::parse();

    let command = if cli.command.is_empty() {
        None
    } else {
        Some(cli.command)
    };

    let saved_stdin = redirect_stdin_to_tty()?;

    let mut terminal = ratatui::init();
    let (rx, _child) = input::spawn_line_reader(command, saved_stdin);

    let mut app = App::new(cli.max_lines);
    let result = run_app(&mut terminal, &mut app, rx);

    ratatui::restore();
    result
}

fn run_app(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<String>,
) -> io::Result<()> {
    loop {
        // Receive new lines (non-blocking)
        while let Ok(line) = rx.try_recv() {
            app.add_line(line);
        }

        terminal.draw(|frame| app.render(frame))?;

        if app.should_quit {
            return Ok(());
        }

        // Poll for events with a short timeout
        if app.poll_events()? {
            let event = app.next_event()?;
            let area = terminal.get_frame().area();
            app.handle_event(event, area);
        }
    }
}
