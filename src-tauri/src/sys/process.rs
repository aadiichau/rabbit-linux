use std::ffi::OsStr;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use tokio::process::{Child, Command};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A child process contained in a Windows Job Object or a Unix process group.
/// Dropping it (or calling `terminate`) kills the tree — yt-dlp.exe is a PyInstaller
/// bootloader that spawns a second process, so killing only the direct child is not enough.
pub struct ContainedChild {
    pub child: Child,
    job: Arc<Job>,
}

impl ContainedChild {
    /// A handle that can kill the process tree while `child` is borrowed elsewhere.
    pub fn killer(&self) -> Killer {
        Killer(self.job.clone())
    }
}

#[derive(Clone)]
pub struct Killer(Arc<Job>);

impl Killer {
    pub fn kill(&self) {
        self.0.terminate();
    }
}

/// Builds a command with no console window, no inherited stdin, and piped output.
/// Arguments are always passed as an argv array; no shell is involved.
pub fn command<I, S>(program: &Path, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

pub fn spawn_contained(mut cmd: Command) -> std::io::Result<ContainedChild> {
    #[cfg(windows)]
    let job = Job::new()?;
    let child = cmd.spawn()?;
    #[cfg(windows)]
    if let Some(handle) = child.raw_handle() {
        job.assign(handle as isize);
    }
    #[cfg(unix)]
    let job = Job(child.id().ok_or_else(|| std::io::Error::other("child has no process ID"))? as i32);
    Ok(ContainedChild { child, job: Arc::new(job) })
}

#[cfg(windows)]
struct Job(isize);

// SAFETY: a job handle is a kernel object handle; it may be used from any thread.
#[cfg(windows)]
unsafe impl Send for Job {}
#[cfg(windows)]
unsafe impl Sync for Job {}

#[cfg(windows)]
impl Job {
    fn new() -> std::io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            Ok(Job(h as isize))
        }
    }

    fn assign(&self, process: isize) {
        unsafe {
            windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(
                self.0 as _,
                process as _,
            );
        }
    }

    fn terminate(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0 as _, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[tokio::test]
    async fn cancellation_kills_the_child_and_its_descendants() {
        let mut proc = spawn_contained(command(Path::new("/bin/sh"), ["-c", "sleep 60 & echo $!; wait"])).unwrap();
        let mut lines = BufReader::new(proc.child.stdout.take().unwrap()).lines();
        let pid: i32 = lines.next_line().await.unwrap().unwrap().parse().unwrap();
        // SAFETY: signal 0 only checks whether the process exists.
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        proc.killer().kill();
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), proc.child.wait()).await.unwrap().unwrap();
        assert!(!status.success());
        // An orphan may briefly remain as a zombie until the system reaps it.
        for _ in 0..100 {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
            if stat.is_err() || stat.unwrap().split_once(") ").is_some_and(|(_, s)| s.starts_with('Z')) { return; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("descendant {pid} survived cancellation");
    }

    #[tokio::test]
    async fn arguments_are_passed_without_shell_interpretation() {
        let proc = spawn_contained(command(Path::new("/usr/bin/printf"), ["%s", "a b;$(echo unsafe)"])).unwrap();
        let out = proc.child.wait_with_output().await.unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, b"a b;$(echo unsafe)");
    }
}

#[cfg(unix)]
struct Job(i32);

#[cfg(unix)]
impl Job {
    fn terminate(&self) {
        // SAFETY: a negative PID targets only the process group created for this child.
        unsafe { libc::kill(-self.0, libc::SIGKILL); }
    }
}

#[cfg(unix)]
impl Drop for Job {
    fn drop(&mut self) {
        self.terminate();
    }
}
