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

    /// Output app state as NDJSON after each input line (for testing)
    #[arg(long)]
    json_output: bool,

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

    if cli.json_output {
        return run_json_mode(cli.max_lines);
    }

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let _guard = rt.enter();

    let command = if cli.command.is_empty() {
        None
    } else {
        Some(cli.command)
    };

    let saved_stdin = redirect_stdin_to_tty()?;

    let mut terminal = ratatui::init();
    let (rx, _child) = if command.is_none() && saved_stdin.is_none() {
        // No input source (TTY without pipe) — skip line reader to avoid fd conflict with crossterm
        let (_, rx) = tokio::sync::mpsc::unbounded_channel();
        (rx, None)
    } else {
        input::spawn_line_reader(command, saved_stdin)
    };

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

fn parse_cmd_event(cmd: &str) -> Option<crossterm::event::Event> {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    let (key, modifiers) = if let Some(rest) = cmd.strip_prefix("C-") {
        let c = rest.chars().next()?;
        (KeyCode::Char(c), KeyModifiers::CONTROL)
    } else {
        match cmd {
            "Enter" => (KeyCode::Enter, KeyModifiers::NONE),
            "Esc" => (KeyCode::Esc, KeyModifiers::NONE),
            "Backspace" => (KeyCode::Backspace, KeyModifiers::NONE),
            "Up" => (KeyCode::Up, KeyModifiers::NONE),
            "Down" => (KeyCode::Down, KeyModifiers::NONE),
            "Tab" => (KeyCode::Tab, KeyModifiers::NONE),
            s if s.len() == 1 => {
                let c = s.chars().next()?;
                (KeyCode::Char(c), KeyModifiers::NONE)
            }
            _ => return None,
        }
    };

    Some(Event::Key(KeyEvent::new(key, modifiers)))
}

fn run_json_mode(max_lines: usize) -> io::Result<()> {
    use std::io::{BufRead, Write};

    let mut app = App::new(max_lines);
    let area = ratatui::layout::Rect::new(0, 0, 80, 24);
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if let Some(cmd) = line.strip_prefix("CMD:") {
            if let Some(event) = parse_cmd_event(cmd) {
                app.handle_event(event, area);
            }
        } else {
            app.add_line(line);
        }

        app.update_auto_scroll(area.height as usize);

        let state = app.snapshot();
        let json = serde_json::to_string(&state).unwrap();
        writeln!(stdout, "{}", json)?;
        stdout.flush()?;

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
