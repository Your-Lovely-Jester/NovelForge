//! Child-process policy shared by every external engine.
//!
//! Two Windows-only hazards are handled here instead of at each call site.
//!
//! * NovelForge is a windowed app, so it owns no console. Windows therefore
//!   gives every console child a brand-new console window, which is the
//!   `cmd.exe` flash seen on start-up (tool probes), on metadata fetches and for
//!   the whole duration of a download. Engine launchers are `.cmd` shims, so the
//!   window comes from `cmd.exe` one level below us. `CREATE_NO_WINDOW` hides it.
//! * `std::fs::canonicalize` returns verbatim `\\?\C:\...` paths. Rust is happy
//!   with them, but a Python tool that interpolates one into a SQLite URL
//!   (`Path.resolve().as_posix()` -> `sqlite://///?/C:/...`) fails with
//!   "unable to open database file". Paths stay verbatim inside the app and are
//!   simplified only when they cross a process boundary.
//!
//! Windows has no `SIGSTOP`, and pausing only the launcher would leave the real
//! crawler running (`lncrawl` is started through a `.cmd` shim, so `cmd.exe` is
//! merely the parent of `python.exe`). [`suspend_process_tree`] therefore walks
//! the live process tree and suspends every member; [`resume_process_tree`]
//! releases the same tree.

use std::path::{Path, PathBuf};

/// `CreateProcess` flag: give the child no console window at all.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Suppress the console window a child would otherwise open. No-op elsewhere.
pub fn hide_console_std(command: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Same as [`hide_console_std`] for the async spawner.
pub fn hide_console(command: &mut tokio::process::Command) -> &mut tokio::process::Command {
    hide_console_std(command.as_std_mut());
    command
}

/// Remove the Windows verbatim prefix so external tools see the path Explorer
/// shows. Non-Windows platforms, and namespaces that must keep their prefix
/// (volume GUIDs, device paths), are returned unchanged.
pub fn native_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        const PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
        const UNC: &[u16] = &[b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16];

        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if let Some(rest) = wide.strip_prefix(PREFIX) {
            // `\\?\UNC\server\share` is the verbatim spelling of `\\server\share`.
            if let Some(unc) = rest.strip_prefix(UNC) {
                let mut native = vec![b'\\' as u16, b'\\' as u16];
                native.extend_from_slice(unc);
                return PathBuf::from(OsString::from_wide(&native));
            }
            let letter = rest.first().copied().unwrap_or(0);
            let drive_letter = letter < 0x80 && (letter as u8).is_ascii_alphabetic();
            if drive_letter && rest.get(1) == Some(&(b':' as u16)) {
                return PathBuf::from(OsString::from_wide(rest));
            }
        }
        path.to_path_buf()
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// Suspends every process in the tree rooted at `pid`, parents first, and
/// reports how many were stopped. All-or-nothing: if the root cannot be
/// suspended, anything already stopped is resumed and an error is returned, so
/// a paused job never leaves half a crawler running.
#[cfg(windows)]
pub fn suspend_process_tree(pid: u32) -> Result<usize, String> {
    apply_to_tree(pid, false)
}

/// Releases every process in the tree rooted at `pid`. Processes that exited
/// while paused are skipped; the root is required to still exist.
#[cfg(windows)]
pub fn resume_process_tree(pid: u32) -> Result<usize, String> {
    apply_to_tree(pid, true)
}

/// `NtSuspendProcess`/`NtResumeProcess` are the only APIs that stop every
/// thread of a process at once. They live in `ntdll` but are not in the import
/// libraries, so they are resolved at runtime instead of linked.
#[cfg(windows)]
type NtProcessCall = unsafe extern "system" fn(windows_sys::Win32::Foundation::HANDLE) -> i32;

#[cfg(windows)]
fn nt_process_call(resume: bool) -> Result<NtProcessCall, String> {
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::HMODULE;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

    static SUSPEND: OnceLock<Option<NtProcessCall>> = OnceLock::new();
    static RESUME: OnceLock<Option<NtProcessCall>> = OnceLock::new();

    let slot = if resume { &RESUME } else { &SUSPEND };
    let resolved = slot.get_or_init(|| unsafe {
        let module: HMODULE = GetModuleHandleA(b"ntdll.dll\0".as_ptr());
        if module.is_null() {
            return None;
        }
        let name: &[u8] = if resume {
            b"NtResumeProcess\0"
        } else {
            b"NtSuspendProcess\0"
        };
        let address = GetProcAddress(module, name.as_ptr())?;
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            NtProcessCall,
        >(address))
    });

    resolved.ok_or_else(|| {
        format!(
            "ntdll.dll does not expose {}",
            if resume {
                "NtResumeProcess"
            } else {
                "NtSuspendProcess"
            }
        )
    })
}

/// Applies one NT call to every member of the tree. Returns how many processes
/// were affected. `psutil`-style tools use the same trick; Windows itself has
/// no supported pause API before Windows 11's `SuspendProcess` wrappers.
#[cfg(windows)]
fn apply_to_tree(pid: u32, resume: bool) -> Result<usize, String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SUSPEND_RESUME};

    if pid == 0 {
        return Err("No crawl process is attached to this download".to_string());
    }
    let call = nt_process_call(resume)?;
    let mut done: Vec<u32> = Vec::new();
    let mut root_error = None;

    for target in process_tree(pid) {
        let handle = unsafe { OpenProcess(PROCESS_SUSPEND_RESUME, 0, target) };
        if handle.is_null() {
            // A descendant may have exited between the snapshot and this call.
            if target == pid {
                root_error = Some(format!(
                    "Windows did not allow access to the crawl process (pid {pid})"
                ));
                break;
            }
            continue;
        }
        let status = unsafe { call(handle) };
        unsafe { CloseHandle(handle) };
        if status >= 0 {
            done.push(target);
        } else if target == pid {
            root_error = Some(format!(
                "Windows refused to {} the crawl process (status {status:#x})",
                if resume { "resume" } else { "suspend" }
            ));
            break;
        }
    }

    if let Some(error) = root_error {
        // Undo a partial suspend; resuming something that was never stopped is
        // harmless, so the cleanup does not need its own bookkeeping.
        if !resume {
            let _ = apply_to_tree(pid, true);
        }
        return Err(error);
    }
    if done.is_empty() {
        return Err(format!("The crawl process (pid {pid}) has already exited"));
    }
    Ok(done.len())
}

/// Live process ids in the tree rooted at `pid`, parents before children, so a
/// suspended parent cannot spawn a replacement while its subtree is stopped.
#[cfg(windows)]
fn process_tree(pid: u32) -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    if pid == 0 {
        return Vec::new();
    }

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
        return vec![pid];
    }

    let mut links: Vec<(u32, u32)> = Vec::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while has_entry {
        links.push((entry.th32ProcessID, entry.th32ParentProcessID));
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };

    let mut ordered = vec![pid];
    let mut cursor = 0;
    while cursor < ordered.len() {
        let parent = ordered[cursor];
        for (child, child_parent) in &links {
            if *child_parent == parent && *child != parent && !ordered.contains(child) {
                ordered.push(*child);
            }
        }
        cursor += 1;
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(windows))]
    #[test]
    fn native_path_is_unchanged_off_windows() {
        let path = Path::new("/srv/novelforge/.novelforge-work/job");
        assert_eq!(native_path(path), path);
    }

    #[cfg(windows)]
    #[test]
    fn native_path_strips_the_verbatim_prefix() {
        assert_eq!(
            native_path(Path::new(
                r"\\?\C:\Users\Example\NovelForge\.novelforge-work\job"
            )),
            PathBuf::from(r"C:\Users\Example\NovelForge\.novelforge-work\job")
        );
        assert_eq!(
            native_path(Path::new(r"\\?\UNC\server\share\libraries")),
            PathBuf::from(r"\\server\share\libraries")
        );
    }

    #[cfg(windows)]
    #[test]
    fn native_path_keeps_other_namespaces() {
        let volume = Path::new(r"\\?\Volume{2e2f0a3f-0000-0000-0000-100000000000}\library");
        assert_eq!(native_path(volume), volume);
        let plain = Path::new(r"C:\Users\Example\NovelForge");
        assert_eq!(native_path(plain), plain);
    }

    /// `cmd.exe` is the launcher and `ping.exe` the worker, mirroring the
    /// `.cmd` shim that starts lncrawl. Suspending only the launcher would leave
    /// the worker running, so the tree must stop both.
    #[cfg(windows)]
    #[test]
    fn pause_stops_the_whole_tree_and_resume_releases_it() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let mut child = Command::new("cmd")
            .args(["/C", "ping", "-n", "3", "127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a two-level tree");
        let pid = child.id();
        std::thread::sleep(Duration::from_millis(400));

        let stopped = suspend_process_tree(pid).expect("suspend the tree");
        assert!(
            stopped >= 2,
            "expected launcher and worker, suspended {stopped}"
        );

        // `ping -n 3` finishes in about two seconds. Still running after 2.5 s
        // of wall clock is only possible if every process in the tree stopped.
        std::thread::sleep(Duration::from_millis(2_500));
        assert!(
            child.try_wait().expect("poll").is_none(),
            "the tree kept running while paused"
        );

        let released = resume_process_tree(pid).expect("resume the tree");
        assert!(released >= 2, "resumed only {released} process(es)");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if child.try_wait().expect("poll").is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the tree did not finish after resume"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[cfg(windows)]
    #[test]
    fn pause_refuses_a_process_that_is_already_gone() {
        // Pid 0 is never a real process; the guard must refuse it before any
        // syscall instead of silently reporting a paused download.
        assert!(suspend_process_tree(0).is_err());
        assert!(resume_process_tree(0).is_err());
    }
}
