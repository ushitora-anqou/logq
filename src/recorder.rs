use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

pub struct Recorder {
    tx: Option<mpsc::Sender<String>>,
    path: PathBuf,
    thread: Option<thread::JoinHandle<()>>,
}

impl Recorder {
    pub fn start(path: PathBuf) -> std::io::Result<Self> {
        let file = File::create(&path)?;
        let (tx, rx) = mpsc::channel::<String>();

        let handle = thread::Builder::new()
            .name("logq-recorder".into())
            .spawn(move || {
                let mut writer = BufWriter::new(file);
                let mut count = 0u64;
                while let Ok(line) = rx.recv() {
                    if writeln!(writer, "{}", line).is_err() {
                        break;
                    }
                    count += 1;
                    if count.is_multiple_of(100) {
                        let _ = writer.flush();
                    }
                }
                let _ = writer.flush();
            })?;

        Ok(Self {
            tx: Some(tx),
            path,
            thread: Some(handle),
        })
    }

    pub fn record(&self, line: &str) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(line.to_string());
        }
    }

    pub fn stop(&mut self) {
        // Drop the sender to close the channel, then join the writer thread.
        self.tx.take();
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_recorder_writes_lines() {
        let dir = std::env::temp_dir().join("logq_test_recorder");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.log");

        {
            let recorder = Recorder::start(path.clone()).unwrap();
            recorder.record("line1");
            recorder.record("line2");
            recorder.record("line3");
            drop(recorder);
        }

        let mut f = std::fs::File::open(&path).unwrap();
        let mut content = String::new();
        f.read_to_string(&mut content).unwrap();
        assert_eq!(content, "line1\nline2\nline3\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_recorder_stop_flushes() {
        let dir = std::env::temp_dir().join("logq_test_recorder_stop");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.log");

        {
            let mut recorder = Recorder::start(path.clone()).unwrap();
            recorder.record("hello");
            recorder.stop();

            // After stop, file should be flushed
            let mut f = std::fs::File::open(&path).unwrap();
            let mut content = String::new();
            f.read_to_string(&mut content).unwrap();
            assert_eq!(content, "hello\n");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_recorder_path() {
        let dir = std::env::temp_dir().join("logq_test_recorder_path");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.log");

        let recorder = Recorder::start(path.clone()).unwrap();
        assert_eq!(recorder.path(), path);
        drop(recorder);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_recorder_creates_file() {
        let dir = std::env::temp_dir().join("logq_test_recorder_create");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("new.log");

        let recorder = Recorder::start(path.clone()).unwrap();
        assert!(path.exists());
        drop(recorder);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
