# logq

A terminal UI viewer for NDJSON (newline-delimited JSON) and plain text streams, written in Rust.

logq reads lines from stdin or a spawned command and displays them in an interactive TUI with syntax highlighting, filtering, and vim-style navigation.

## Features

- **Live tailing**: Lines stream in real-time like `tail -f`, with auto-scroll that pauses when you navigate away and resumes with `G`
- **JSON syntax highlighting**: Color-coded keys, strings, numbers, booleans, and null values
- **Pretty-printed detail view**: Press Enter to expand a line into a readable, indented JSON view
- **Substring filtering**: Type `/pattern` to show only matching lines
- **Non-JSON support**: Lines that aren't valid JSON are displayed as-is
- **Vim-style scrolling**: `C-d`, `C-u`, `C-f`, `C-b`, `C-e`, `C-y` all move both the viewport and selection
- **Memory-bounded**: Configurable line limit discards oldest lines when exceeded

## Usage

### Read from stdin (pipe)

```sh
command-producing-ndjson | logq
```

### Run a command directly

```sh
logq -- command arg1 arg2 ...
```

### Options

```
--max-lines <N>  Maximum number of lines to keep in memory (default: 10000)
```

## Keybindings

### List view

| Key           | Action                          |
|---------------|---------------------------------|
| `j` / `Down`  | Move selection down             |
| `k` / `Up`    | Move selection up               |
| `Enter`       | Open detail view for selection  |
| `/`           | Start filter input              |
| `G`           | Jump to latest line (resume auto-scroll) |
| `C-d`         | Scroll down half page           |
| `C-u`         | Scroll up half page             |
| `C-f`         | Scroll down full page           |
| `C-b`         | Scroll up full page             |
| `C-e`         | Scroll down one line            |
| `C-y`         | Scroll up one line              |
| `C-c` `C-c`   | Quit (press twice quickly)      |

### Filter input mode

| Key           | Action                          |
|---------------|---------------------------------|
| `Enter`       | Apply filter                    |
| `Esc`         | Cancel filter input             |
| `Backspace`   | Delete last character           |
| `<char>`      | Append character to filter      |

### Detail view

| Key           | Action                          |
|---------------|---------------------------------|
| `Backspace` / `Esc` | Return to list view       |
| `j` / `Down`  | Scroll down                     |
| `k` / `Up`    | Scroll up                       |
| `C-d`         | Scroll down half page           |
| `C-u`         | Scroll up half page             |
| `C-f`         | Scroll down full page           |
| `C-b`         | Scroll up full page             |
| `C-e`         | Scroll down one line            |
| `C-y`         | Scroll up one line              |
| `C-c` `C-c`   | Quit (press twice quickly)      |

## Examples

```sh
# View JSON logs from a service
kubectl logs -f my-pod | logq

# Run a command and view its output
logq -- my-script.sh

# Limit to 5000 lines
cat large-file.ndjson | logq --max-lines 5000
```

## License

MIT
