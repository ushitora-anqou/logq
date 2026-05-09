# logq

A terminal UI viewer for NDJSON (newline-delimited JSON) and plain text streams, written in Rust.

logq reads lines from stdin or a spawned command and displays them in an interactive TUI with syntax highlighting, regex filtering, timestamps, and vim-style navigation.

## Features

- **Live tailing**: Lines stream in real-time like `tail -f`, with auto-scroll that pauses when you navigate away and resumes with `G`
- **Timestamps**: Each line shows its received time (`HH:MM:SS.mmm`)
- **JSON syntax highlighting**: Color-coded keys, strings, numbers, booleans, and null values
- **Inline expand**: Press Enter to expand a line into a readable, indented JSON view inline; press `C-o` to expand/collapse all lines
- **Query-based filtering**: Type `/` to enter filter mode with a structured query language supporting literal contains, regex match, and their negations, combinable with AND semantics; JSON key/value conditions support `and`/`or` with parentheses
- **Line formatting**: Use `| line_format "{{ .key }}"` to customize how JSON lines are displayed, substituting field values into a template
- **Breadcrumb bar**: Shows current context (active filter) at the top of the screen
- **Non-JSON support**: Lines that aren't valid JSON are displayed as-is
- **Vim-style scrolling**: `C-d`, `C-u`, `C-f`, `C-b`, `C-e`, `C-y` all move both the viewport and selection
- **Recording**: Save all received lines to a file for later analysis, even when logq is overloaded. Start via `--record` flag or `:record` command at runtime
- **Memory-bounded**: Configurable line limit discards oldest lines when exceeded
- **Internationalization**: UI text (help, shortcuts, status bar, error messages) is automatically displayed in the system locale language. Supported languages: English, Japanese, Chinese (simplified)

## Installation

### Nix

Run directly without installing:

```sh
nix run github:ushitora-anqou/logq -- [options]
```

## Usage

### Read from stdin (pipe)

```sh
command-producing-ndjson | logq
```

### Run a command directly

```sh
logq -- command arg1 arg2 ...
```

### Read from a file

```sh
logq --file logfile.json
```

### Options

```
--max-lines <N>   Maximum number of lines to keep in memory (default: 10000)
--file <PATH>     Read from a file instead of stdin or a command
--record <PATH>   Record all received lines to a file
```

## Keybindings

### List view

| Key           | Action                          |
|---------------|---------------------------------|
| `j` / `Down`  | Move selection down             |
| `k` / `Up`    | Move selection up               |
| `Enter`       | Toggle expand/collapse selected |
| `C-o`         | Expand/collapse all lines       |
| `/`           | Start filter input              |
| `:`           | Enter command mode              |
| `Esc`         | Clear active filter             |
| `G`           | Jump to latest line (resume auto-scroll) |
| `gg`          | Jump to first line              |
| `C-d`         | Scroll down half page           |
| `C-u`         | Scroll up half page             |
| `C-f`         | Scroll down full page           |
| `C-b`         | Scroll up full page             |
| `C-e`         | Scroll down one line            |
| `C-y`         | Scroll up one line              |
| `y`           | Copy selected line to clipboard |
| `c`           | Context view (show all lines around selection, ignoring filter) |
| `C-x`         | Quit                            |

### Filter input mode

| Key           | Action                          |
|---------------|---------------------------------|
| `Enter`       | Apply filter                    |
| `Esc`         | Cancel filter input             |
| `Backspace`   | Delete last character / cancel if empty |
| `<char>`      | Append character to filter      |

After pressing `/`, type a query using the following operators. Plain text without an operator is treated as a substring match (equivalent to `|= "..."`). Values with operators must be enclosed in double quotes. Multiple conditions are combined with AND (space-separated).

| Query                | Meaning                                      |
|----------------------|----------------------------------------------|
| `error`              | Show lines containing "error"                |
| `\|= "error"`        | Show lines containing "error"                |
| `\|~ "err.*timeout"` | Show lines matching the regex                |
| `!= "debug"`         | Show lines NOT containing "debug"            |
| `!~ "err.*"`         | Exclude lines matching the regex             |
| `\|= "error" != "timeout"` | Show lines containing "error" AND not containing "timeout" |

### JSON key/value filters

Filter by JSON fields using `| key op value`. Values can be strings (`"..."`), numbers, booleans (`true`/`false`), or `null`. Supports nested keys with dot notation (`user.name`).

| Query                          | Meaning                                          |
|--------------------------------|--------------------------------------------------|
| `\| level = "error"`            | JSON where `level` equals `"error"`              |
| `\| count != 0`                 | JSON where `count` is not 0                      |
| `\| msg =~ "err.*"`             | JSON where `msg` matches the regex               |
| `\| active = true`              | JSON where `active` is true                      |
| `\| user.name = "alice"`        | JSON where nested `user.name` equals `"alice"`   |

JSON key conditions support `and`, `or`, and parentheses for grouping:

| Query                          | Meaning                                          |
|--------------------------------|--------------------------------------------------|
| `\| level = "error" and count > 0` | Both conditions must match                    |
| `\| level = "error" or level = "warn"` | Either condition matches                  |
| `\| (level = "error" or level = "warn") and active = true` | Grouped with parens |

Plain-text conditions (`|=`, `!=`, `|~`, `!~`) cannot use `and`/`or` — they are always ANDed at the top level.

### Line formatting

Use `| line_format "template"` to customize how JSON lines are displayed. The template uses `{{ .key }}` placeholders that are replaced with the corresponding JSON field values. Nested keys are supported with dot notation.

| Query                                      | Meaning                                              |
|--------------------------------------------|------------------------------------------------------|
| `\| line_format "{{ .msg }}"`               | Display only the `msg` field                        |
| `\| line_format "{{ .a }} / {{ .b }}"`      | Display `a` and `b` fields separated by ` / `       |
| `\|= "error" \| line_format "{{ .msg }}"`  | Filter for "error" AND format output to show `msg`  |

Non-JSON lines are displayed as-is. Missing keys are replaced with empty strings.

### Command mode

Press `:` to enter command mode. Available commands:

| Command            | Action                              |
|--------------------|-------------------------------------|
| `:record <PATH>`   | Start recording lines to a file     |
| `:stoprecord`      | Stop recording                      |
| `:stop`            | Stop recording (alias)              |

Recording writes all received lines to the specified file in a background thread, so it continues even when logq is overloaded. The titlebar shows `[REC: path]` while recording is active.

## Examples

```sh
# View JSON logs from a service
kubectl logs -f my-pod | logq

# Run a command and view its output
logq -- my-script.sh

# Limit to 5000 lines
cat large-file.ndjson | logq --max-lines 5000

# Record all lines to a file (for later debugging)
kubectl logs -f my-pod | logq --record /tmp/captured.log

# Start/stop recording at runtime
# (inside logq) type :record /tmp/debug.log to start
# (inside logq) type :stoprecord to stop

# Filter with query language
# (inside logq) type /|~ "err.*timeout" to show only matching lines

# Combine conditions
# (inside logq) type /|= "error" != "timeout" to show errors excluding timeouts
```

## License

MIT
