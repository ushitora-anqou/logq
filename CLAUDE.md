# CLAUDE.md

## IMPORTANT

- **TDD**: When adding a feature or fixing a bug, write a failing test first, confirm it fails, then implement the fix.
- **Pre-commit formatting**: Always run `make fmt` before `git commit`.
- **Update documentation**: When adding or modifying a feature, update README.md and the `--help` output (clap `#[command]`/`#[arg]` attributes in `main.rs`) accordingly.

## Project Overview

logq is a terminal UI (TUI) viewer for NDJSON and plain text streams, written in Rust. It reads lines from stdin or a spawned command and displays them in an interactive TUI with syntax highlighting, regex filtering, timestamps, and vim-style navigation.

## Build & Development Commands

```bash
cargo build                  # Debug build
cargo build --release        # Release build
cargo test                   # Run all tests (unit + integration)
cargo test --lib             # Run unit tests only
cargo test --test integration # Run integration tests only
cargo fmt                    # Format Rust code
taplo fmt Cargo.toml taplo.toml deny.toml  # Format TOML files
cargo clippy --all-targets -- -D warnings   # Lint (warnings as errors)
make all                     # fmt + lint + test + build
```

## Architecture

5 source files, no nested modules:

- **`lib.rs`** — Library crate root. Re-exports `app`, `highlight`, and `input` modules for use by both the binary and integration tests.
- **`main.rs`** — Entry point. CLI parsing (clap), stdin-to-TTY redirect via `libc::dup/dup2`, Tokio runtime setup, main TUI event loop (`run_app`).
- **`app.rs`** — Core state and logic. `App` struct holds all state (lines, selection, scroll, filter, view mode). Handles keyboard events with vim bindings, renders TUI via ratatui (breadcrumb + list/detail view + status bar).
- **`highlight.rs`** — JSON syntax highlighting. Pretty-prints valid JSON with color-coded spans. `HighlightColors` holds the color scheme. `find_string_end` handles escape sequences. Invalid JSON falls back to plain text.
- **`input.rs`** — Async line reader. Spawns commands or reads stdin via Tokio, sends lines through `mpsc::UnboundedReceiver`.

### Data Flow

```
stdin/command → input.rs (tokio spawn) → mpsc channel → main loop (try_recv) → app.add_line()
```

### Key Design Decisions

- **TTY redirect**: When stdin is a pipe, `dup(0)` saves it and `dup2(tty, 0)` replaces fd 0 with `/dev/tty` so crossterm can read keyboard input while data comes from the pipe.
- **Auto-scroll**: Tracks whether the user is at the latest line. `G` re-enables auto-scroll; any manual navigation away from the last line disables it.
- **Filter query language**: After `/`, users type structured queries like `|= "foo" != "bar"`. Four operators (`|=`, `|~`, `!=`, `!~`) with quoted values, combined with AND semantics. Parsed by `parse_filter_query()`; invalid queries show errors in the status bar with input preserved for editing.
- **Memory-bounded**: `max_lines` (default 10,000) drops oldest entries via `Vec::remove(0)`.

## Testing

- Unit tests are `#[cfg(test)]` modules inside each source file.
- Integration tests in `tests/integration.rs` use ratatui's `TestBackend` and inject real `crossterm::event::Event::Key` events directly into `App::handle_event()`. They render via `App::render()` and assert on the rendered buffer contents (what the user would see on screen).
- `test_tui_mode_with_command_no_panic` runs logq under `script` to verify the TUI path doesn't panic.
