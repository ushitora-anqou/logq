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

/// Spawns a line reader that sends each line through the channel.
/// For stdin mode, reads from the given file (or tokio::stdin if None).
/// For command mode, spawns the command and reads both stdout and stderr.
/// Returns (receiver, child_pid, task_handle).
/// The task_handle is the exit monitor (command mode) or reader (stdin mode).
pub fn spawn_line_reader(
    command: Option<Vec<String>>,
    stdin_file: Option<File>,
) -> (
    mpsc::UnboundedReceiver<InputLine>,
    Option<u32>,
    Option<JoinHandle<()>>,
) {
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

            (rx, Some(pid), Some(exit_handle))
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
            (rx, None, Some(handle))
        }
    }
}

async fn read_lines_with_source<R: AsyncBufReadExt + Unpin>(
    reader: R,
    tx: mpsc::UnboundedSender<InputLine>,
    source: LineSource,
) {
    let mut lines = reader.lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
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
}
