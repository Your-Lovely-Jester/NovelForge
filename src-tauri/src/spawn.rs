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
}
