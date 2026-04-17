# logq

A terminal UI viewer for NDJSON (newline-delimited JSON) and plain text streams, written in Rust.

logq reads lines from stdin or a spawned command and displays them in an interactive TUI with syntax highlighting, regex filtering, timestamps, and vim-style navigation.

## Features

- **Live tailing**: Lines stream in real-time like `tail -f`, with auto-scroll that pauses when you navigate away and resumes with `G`
- **Timestamps**: Each line shows its received time (`HH:MM:SS.mmm`)
- **JSON syntax highlighting**: Color-coded keys, strings, numbers, booleans, and null values
- **Pretty-printed detail view**: Press Enter to expand a line into a readable, indented JSON view
- **Query-based filtering**: Type `/` to enter filter mode with a structured query language supporting literal contains, regex match, and their negations, combinable with AND semantics; JSON key/value conditions support `and`/`or` with parentheses
- **Breadcrumb bar**: Shows current context (detail view, active filter) at the top of the screen
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
| `Esc`         | Clear active filter             |
| `G`           | Jump to latest line (resume auto-scroll) |
| `C-d`         | Scroll down half page           |
| `C-u`         | Scroll up half page             |
| `C-f`         | Scroll down full page           |
| `C-b`         | Scroll up full page             |
| `C-e`         | Scroll down one line            |
| `C-y`         | Scroll up one line              |
| `C-x`         | Quit                            |

### Filter input mode

| Key           | Action                          |
|---------------|---------------------------------|
| `Enter`       | Apply filter                    |
| `Esc`         | Cancel filter input             |
| `Backspace`   | Delete last character / cancel if empty |
| `<char>`      | Append character to filter      |

After pressing `/`, type a query using the following operators. Values must be enclosed in double quotes. Multiple conditions are combined with AND (space-separated).

| Query                | Meaning                                      |
|----------------------|----------------------------------------------|
| `|= "error"`         | Show lines containing "error"                |
| `|~ "err.*timeout"`  | Show lines matching the regex                |
| `!= "debug"`         | Show lines NOT containing "debug"            |
| `!~ "err.*"`         | Exclude lines matching the regex             |
| `|= "error" != "timeout"` | Show lines containing "error" AND not containing "timeout" |

### JSON key/value filters

Filter by JSON fields using `| key op value`. Values can be strings (`"..."`), numbers, booleans (`true`/`false`), or `null`. Supports nested keys with dot notation (`user.name`).

| Query                          | Meaning                                          |
|--------------------------------|--------------------------------------------------|
| `| level = "error"`            | JSON where `level` equals `"error"`              |
| `| count != 0`                 | JSON where `count` is not 0                      |
| `| msg =~ "err.*"`             | JSON where `msg` matches the regex               |
| `| active = true`              | JSON where `active` is true                      |
| `| user.name = "alice"`        | JSON where nested `user.name` equals `"alice"`   |

JSON key conditions support `and`, `or`, and parentheses for grouping:

| Query                          | Meaning                                          |
|--------------------------------|--------------------------------------------------|
| `| level = "error" and count > 0` | Both conditions must match                    |
| `| level = "error" or level = "warn"` | Either condition matches                  |
| `| (level = "error" or level = "warn") and active = true` | Grouped with parens |

Plain-text conditions (`|=`, `!=`, `|~`, `!~`) cannot use `and`/`or` — they are always ANDed at the top level.

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
| `C-x`         | Quit                            |

## Examples

```sh
# View JSON logs from a service
kubectl logs -f my-pod | logq

# Run a command and view its output
logq -- my-script.sh

# Limit to 5000 lines
cat large-file.ndjson | logq --max-lines 5000

# Filter with query language
# (inside logq) type /|~ "err.*timeout" to show only matching lines

# Combine conditions
# (inside logq) type /|= "error" != "timeout" to show errors excluding timeouts
```

## License

MIT
