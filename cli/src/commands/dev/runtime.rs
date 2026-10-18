//! The log file `a4 dev up` writes (`.arete/dev/a4.log`) and the check that
//! the process a state file names is still alive.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

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

/// Whether `pid` is a live process. Unknown/access-denied states are treated
/// as alive so status never silently discards a potentially running session.
pub fn pid_alive(pid: u32) -> bool {
    process_alive(pid).unwrap_or(true)
}

fn process_alive(pid: u32) -> Result<bool> {
    if pid == 0 {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        let pid: libc::pid_t = pid.try_into().context("Invalid process id")?;
        // SAFETY: a positive pid and signal 0 only check process existence.
        if unsafe { libc::kill(pid, 0) } == 0 {
            // An exited child can retain its pid until its parent reaps it.
            // It no longer owns servers/ports and must not block dev down.
            #[cfg(target_os = "linux")]
            let zombie = fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|stat| {
                    stat.rsplit_once(") ")
                        .map(|(_, fields)| fields.starts_with("Z "))
                })
                .unwrap_or(false);
            #[cfg(not(target_os = "linux"))]
            let zombie = std::process::Command::new("/bin/ps")
                .args(["-p", &pid.to_string(), "-o", "stat="])
                .output()
                .is_ok_and(|output| {
                    output.status.success()
                        && String::from_utf8_lossy(&output.stdout)
                            .trim_start()
                            .starts_with('Z')
                });
            return Ok(!zombie);
        }
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ESRCH) => Ok(false),
            Some(libc::EPERM) => Ok(true),
            _ => Err(error).context("Failed to check dev process"),
        }
    }
    #[cfg(windows)]
    {
        match windows::Process::open(pid)? {
            Some(process) => process.alive(),
            None => Ok(false),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        bail!("Process checks are unsupported on this platform")
    }
}

/// Stop the recorded dev process and confirm it exited before deleting state.
/// Windows stops its child processes too (including the Node handler host).
pub fn stop_process(pid: u32, timeout: Duration) -> Result<()> {
    if pid == 0 || pid == std::process::id() {
        bail!("Refusing to stop invalid/self process {pid}");
    }
    #[cfg(unix)]
    {
        if !process_alive(pid)? {
            return Ok(());
        }
        let native: libc::pid_t = pid.try_into().context("Invalid process id")?;
        // SAFETY: a validated positive pid recorded by this project's dev up.
        if unsafe { libc::kill(native, libc::SIGTERM) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error).context("Failed to stop dev process");
            }
        }
        let deadline = Instant::now() + timeout;
        while process_alive(pid)? {
            if Instant::now() >= deadline {
                bail!("Dev process {pid} did not stop; session files were retained");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        windows::stop(pid, timeout)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = timeout;
        bail!("Stopping dev processes is unsupported on this platform")
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    pub(super) struct Process(HANDLE);
    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: this wrapper owns one valid OpenProcess handle.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    impl Process {
        pub(super) fn open(pid: u32) -> Result<Option<Self>> {
            // SAFETY: querying a pid with synchronization-only access.
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            if !handle.is_null() {
                return Ok(Some(Self(handle)));
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                return Ok(None);
            }
            Err(error).context("Failed to open dev process")
        }
        pub(super) fn alive(&self) -> Result<bool> {
            // SAFETY: the owned handle stays valid for this zero-time wait.
            match unsafe { WaitForSingleObject(self.0, 0) } {
                WAIT_OBJECT_0 => Ok(false),
                WAIT_TIMEOUT => Ok(true),
                _ => Err(std::io::Error::last_os_error()).context("Failed to query dev process"),
            }
        }
    }
    pub(super) fn stop(pid: u32, timeout: Duration) -> Result<()> {
        let Some(process) = Process::open(pid)? else {
            return Ok(());
        };
        if !process.alive()? {
            return Ok(());
        }
        // Use the system utility's absolute path, never a project-local exe.
        let mut directory = vec![0u16; 32768];
        // SAFETY: the buffer has the advertised capacity in UTF-16 units.
        let length =
            unsafe { GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32) } as usize;
        if length == 0 || length >= directory.len() {
            return Err(std::io::Error::last_os_error())
                .context("Failed to locate Windows system directory");
        }
        let taskkill =
            std::path::PathBuf::from(std::ffi::OsString::from_wide(&directory[..length]))
                .join("taskkill.exe");
        let result = std::process::Command::new(taskkill)
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output()
            .context("Failed to start Windows process shutdown")?;
        if !result.status.success() && process.alive()? {
            bail!(
                "Failed to stop dev process {pid}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        // SAFETY: synchronization access was requested, and this handle is
        // kept open through termination and confirmation (including PID reuse).
        let wait = unsafe {
            WaitForSingleObject(
                process.0,
                timeout.as_millis().min((u32::MAX - 1) as u128) as u32,
            )
        };
        match wait {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => bail!("Dev process {pid} did not stop; session files were retained"),
            _ => Err(std::io::Error::last_os_error())
                .context("Failed to confirm dev process shutdown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "child process fixture, invoked only by the shutdown test"]
    fn process_fixture() {
        std::thread::sleep(Duration::from_secs(60));
    }

    #[test]
    fn shutdown_checks_and_stops_a_child_process() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "commands::dev::runtime::tests::process_fixture",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(pid_alive(pid));
        // Reap concurrently on Unix, where an unreaped child is still a pid.
        let reaper = std::thread::spawn(move || child.wait().unwrap());
        stop_process(pid, Duration::from_secs(5)).unwrap();
        reaper.join().unwrap();
        assert!(!pid_alive(pid));
        stop_process(pid, Duration::from_millis(10)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_accepts_an_exited_unreaped_process() {
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(5);
        while pid_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let alive = pid_alive(pid);
        let stopped = stop_process(pid, Duration::from_millis(20));
        child.wait().unwrap();
        assert!(!alive);
        stopped.unwrap();
    }

    #[test]
    fn appends_lines_and_sees_its_own_process() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs/a4.log");
        let sink = LogSink::open(&path, "a4", false).unwrap();
        sink.line("one");
        sink.clone().line("two");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        assert!(pid_alive(std::process::id()));
    }
}
