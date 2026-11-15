//! Appends timestamped process activity to a log file (async, via tokio).

use std::io;
use std::path::Path;

use chrono::Local;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;

pub struct ActivityLog {
    file: File,
}

impl ActivityLog {
    /// Opens the log in append mode, creating it if needed.
    pub async fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path).await?;
        Ok(Self { file })
    }

    /// Writes one line prefixed with the local time.
    pub async fn write(&mut self, message: &str) -> io::Result<()> {
        let line = format!("{} {message}\n", Local::now().format("%Y-%m-%d %H:%M:%S"));
        self.file.write_all(line.as_bytes()).await?;
        self.file.flush().await
    }
}
