mod app;
mod highlight;
mod input;

use std::io;

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

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    let command = if cli.command.is_empty() {
        None
    } else {
        Some(cli.command)
    };

    let mut terminal = ratatui::init();
    let (rx, _child) = input::spawn_line_reader(command);

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
