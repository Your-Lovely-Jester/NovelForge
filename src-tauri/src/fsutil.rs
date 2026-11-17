//! Durable file writes.
//!
//! Every file that a user can lose work over — `metadata.json`, `index.json`,
//! `chapters.html`, `settings.json`, the job journal and the index cache — is
//! written to a sibling temporary file, flushed, and then replaced. A crash or
//! power loss can therefore leave the previous complete file in place instead
//! of a truncated one.
//!
//! On Unix `rename` replaces atomically. On Windows it does not, so the replace
//! step uses `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` there.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Write `bytes` to `path` without ever exposing a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let temp = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    {
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if let Err(error) = replace_file(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

/// Replace `destination` with `temporary`, both in the same directory.
///
/// Callers that did not create the temporary in the destination directory must
/// not use this: a cross-device rename is not atomic.
pub fn replace_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(not(windows))]
    {
        std::fs::rename(temporary, destination)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };

        fn wide(path: &Path) -> Vec<u16> {
            path.as_os_str().encode_wide().chain(Some(0)).collect()
        }
        let from = wide(temporary);
        let to = wide(destination);
        let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
        // SAFETY: both buffers are NUL-terminated and live for the call.
        let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) };
        if ok == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

/// Write text as UTF-8, atomically.
pub fn write_text_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    write_atomic(path, text.as_bytes())
}

/// A temporary sibling path for callers that need to stream rather than build
/// the whole document in memory (the EPUB rewrite).
pub fn sibling_temp(path: &Path, label: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    parent.join(format!(".{name}.{label}.{}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "novelforge-fsutil-{label}-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp() {
        let root = scratch("replace");
        let path = root.join("metadata.json");
        write_text_atomic(&path, "first").unwrap();
        write_text_atomic(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        let leftovers = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_replace_removes_its_temporary() {
        let root = scratch("failed");
        let target = root.join("target");
        std::fs::create_dir(&target).unwrap();
        // Replacing a directory with a file fails; write_atomic must clean up.
        assert!(write_text_atomic(&target, "new").is_err());
        let leftovers = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
