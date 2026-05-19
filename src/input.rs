use std::fs::File;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq)]
pub enum LineSource {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputLine {
    pub text: String,
    pub source: LineSource,
}

/// A spawned line reader producing InputLine values.
/// Holds the receiver, optional child PID (when running a command), and
/// the optional background task handle for the reader/exit-monitor.
pub struct InputReader {
    pub rx: mpsc::UnboundedReceiver<InputLine>,
    pub child_pid: Option<u32>,
    pub task_handle: Option<JoinHandle<()>>,
}

/// Spawns a line reader that sends each line through the channel.
/// For stdin mode, reads from the given file (or tokio::stdin if None).
/// For command mode, spawns the command and reads both stdout and stderr.
/// Returns (receiver, child_pid, task_handle).
/// The task_handle is the exit monitor (command mode) or reader (stdin mode).
pub fn spawn_line_reader(command: Option<Vec<String>>, stdin_file: Option<File>) -> InputReader {
    let (tx, rx) = mpsc::unbounded_channel();

    match command {
        Some(args) => {
            let program = args[0].clone();
            let cmd_args = args[1..].to_vec();
            let mut child = Command::new(&program)
                .args(&cmd_args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .kill_on_drop(true)
                .spawn()
                .expect("Failed to spawn command");

            let pid = child.id().expect("Failed to get child PID");

            let stdout = child.stdout.take().expect("Failed to capture stdout");
            let stderr = child.stderr.take().expect("Failed to capture stderr");

            let stdout_reader = tokio::spawn(read_lines_with_source(
                BufReader::new(stdout),
                tx.clone(),
                LineSource::Stdout,
            ));
            let stderr_reader = tokio::spawn(read_lines_with_source(
                BufReader::new(stderr),
                tx.clone(),
                LineSource::Stderr,
            ));

            // Exit monitor: waits for both readers to finish, then waits on the child.
            // This ensures all output is displayed before the exit message.
            let exit_handle = tokio::spawn(async move {
                let _ = stdout_reader.await;
                let _ = stderr_reader.await;

                let msg = match child.wait().await {
                    Ok(status) => {
                        if status.success() {
                            "process exited successfully".to_string()
                        } else if let Some(code) = status.code() {
                            format!("process exited with code {}", code)
                        } else {
                            "process terminated by signal".to_string()
                        }
                    }
                    Err(e) => format!("failed to wait on process: {}", e),
                };
                let _ = tx.send(InputLine {
                    text: msg,
                    source: LineSource::System,
                });
            });

            InputReader {
                rx,
                child_pid: Some(pid),
                task_handle: Some(exit_handle),
            }
        }
        None => {
            let handle = match stdin_file {
                Some(file) => {
                    let async_file = tokio::fs::File::from_std(file);
                    let reader = BufReader::new(async_file);
                    tokio::spawn(read_lines_with_source(reader, tx, LineSource::Stdout))
                }
                None => {
                    let reader = BufReader::new(tokio::io::stdin());
                    tokio::spawn(read_lines_with_source(reader, tx, LineSource::Stdout))
                }
            };
            InputReader {
                rx,
                child_pid: None,
                task_handle: Some(handle),
            }
        }
    }
}

fn bytes_to_line(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => {
            let mut s = s.to_string();
            if s.ends_with('\r') {
                s.pop();
            }
            s
        }
        Err(_) => {
            let mut out = String::with_capacity(bytes.len());
            for &b in bytes {
                if b == b'\n' || b == b'\r' {
                    continue;
                }
                if (0x20..0x7f).contains(&b) {
                    out.push(b as char);
                } else {
                    out.push_str(&format!("\\x{:02x}", b));
                }
            }
            out
        }
    }
}

async fn read_lines_with_source<R: AsyncBufReadExt + Unpin>(
    reader: R,
    tx: mpsc::UnboundedSender<InputLine>,
    source: LineSource,
) {
    let mut split = reader.split(b'\n');
    loop {
        match split.next_segment().await {
            Ok(Some(bytes)) => {
                let line = bytes_to_line(&bytes);
                if tx
                    .send(InputLine {
                        text: line,
                        source: source.clone(),
                    })
                    .is_err()
                {
                    break;
                }
            }
            Ok(None) => break, // EOF
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn test_read_lines() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let data = "line1\nline2\nline3\n";
        let reader = BufReader::new(Cursor::new(data));
        tokio::spawn(read_lines_with_source(reader, tx, LineSource::Stdout));

        let mut received = Vec::new();
        while let Some(line) = rx.recv().await {
            received.push(line);
        }
        assert_eq!(
            received,
            vec![
                InputLine {
                    text: "line1".to_string(),
                    source: LineSource::Stdout,
                },
                InputLine {
                    text: "line2".to_string(),
                    source: LineSource::Stdout,
                },
                InputLine {
                    text: "line3".to_string(),
                    source: LineSource::Stdout,
                },
            ]
        );
    }

    #[tokio::test]
    async fn test_read_lines_empty() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let data = "";
        let reader = BufReader::new(Cursor::new(data));
        tokio::spawn(read_lines_with_source(reader, tx, LineSource::Stdout));

        let received: Vec<InputLine> = rx.recv().await.into_iter().collect();
        assert!(received.is_empty());
    }

    #[tokio::test]
    async fn test_read_lines_non_utf8_continues_reading() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        // "valid\n<0xc6 0xec 0xd3>\nmore\n" — line 2 contains non-UTF-8 bytes
        let data: Vec<u8> = b"valid\n\xc6\xec\xd3\nmore\n".to_vec();
        let reader = BufReader::new(Cursor::new(data));
        tokio::spawn(read_lines_with_source(reader, tx, LineSource::Stdout));

        let mut received = Vec::new();
        while let Some(line) = rx.recv().await {
            received.push(line);
        }
        assert_eq!(received.len(), 3);
        assert_eq!(received[0].text, "valid");
        assert_eq!(received[1].text, "\\xc6\\xec\\xd3");
        assert_eq!(received[2].text, "more");
    }
}
