//! The log file `a4 dev up` writes (`.arete/dev/a4.log`) and the check that
//! the process a state file names is still alive.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

/// Appends lines to one log file, optionally echoing them.
#[derive(Clone)]
pub struct LogSink {
    file: Arc<Mutex<File>>,
    prefix: String,
    echo: bool,
}

impl LogSink {
    pub fn open(path: &Path, prefix: &str, echo: bool) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("Failed to open {}", path.display()))?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            prefix: prefix.to_string(),
            echo,
        })
    }

    pub fn line(&self, line: &str) {
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{line}");
        }
        if self.echo {
            eprintln!("[{}] {line}", self.prefix);
        }
    }
}

/// Whether `pid` is a live process.
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks for existence.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_lines_and_sees_its_own_process() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs/a4.log");
        let sink = LogSink::open(&path, "a4", false).unwrap();
        sink.line("one");
        sink.clone().line("two");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        #[cfg(unix)]
        assert!(pid_alive(std::process::id()));
    }
}
