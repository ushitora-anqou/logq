use std::fs::File;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Spawns a line reader that sends each line through the channel.
/// For stdin mode, reads from the given file (or tokio::stdin if None).
/// For command mode, spawns the command and reads its stdout.
pub fn spawn_line_reader(
    command: Option<Vec<String>>,
    stdin_file: Option<File>,
) -> (
    mpsc::UnboundedReceiver<String>,
    Option<Child>,
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

            let stdout = child.stdout.take().expect("Failed to capture stdout");
            let reader = BufReader::new(stdout);
            let handle = tokio::spawn(read_lines(reader, tx));

            (rx, Some(child), Some(handle))
        }
        None => {
            let handle = match stdin_file {
                Some(file) => {
                    let async_file = tokio::fs::File::from_std(file);
                    let reader = BufReader::new(async_file);
                    tokio::spawn(read_lines(reader, tx))
                }
                None => {
                    let reader = BufReader::new(tokio::io::stdin());
                    tokio::spawn(read_lines(reader, tx))
                }
            };
            (rx, None, Some(handle))
        }
    }
}

async fn read_lines<R: AsyncBufReadExt + Unpin>(reader: R, tx: mpsc::UnboundedSender<String>) {
    let mut lines = reader.lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if tx.send(line).is_err() {
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
        tokio::spawn(read_lines(reader, tx));

        let mut received = Vec::new();
        while let Some(line) = rx.recv().await {
            received.push(line);
        }
        assert_eq!(received, vec!["line1", "line2", "line3"]);
    }

    #[tokio::test]
    async fn test_read_lines_empty() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let data = "";
        let reader = BufReader::new(Cursor::new(data));
        tokio::spawn(read_lines(reader, tx));

        let received: Vec<String> = rx.recv().await.into_iter().collect();
        assert!(received.is_empty());
    }
}
