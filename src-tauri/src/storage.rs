//! Disk accounting and explicit cleanup (NF-016).
//!
//! Three different trees consume space under the download root:
//!
//! 1. completed structured downloads (kept until the user deletes them),
//! 2. incomplete structured downloads (kept on purpose so they can be retried),
//! 3. per-job crawler work trees under `.novelforge-work` (hidden).
//!
//! This module measures all three and removes only what the user asked for.
//! Completed downloads are never removed here; that stays a per-novel action
//! behind its own confirmation.

use crate::export;
use crate::model::CleanupReport;
use crate::paths;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Hidden per-job crawler tree. The name is also the job id.
pub const WORK_DIR: &str = ".novelforge-work";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkStats {
    pub bytes: u64,
    pub folders: u32,
}

/// Total size of every file below `path`. Symlinks are not followed and are
/// counted as their link size, so a crafted symlink cannot inflate the number.
pub fn directory_bytes(path: &Path) -> u64 {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

/// Library bytes: every visible file under the root. Hidden work trees are
/// excluded so the number matches what the library actually shows.
pub fn library_bytes(root: &Path) -> u64 {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| !paths::is_hidden_path(root, entry.path()))
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

pub fn work_stats(root: &Path) -> WorkStats {
    let work_root = root.join(WORK_DIR);
    let Ok(entries) = std::fs::read_dir(&work_root) else {
        return WorkStats::default();
    };
    let mut stats = WorkStats::default();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        stats.folders += 1;
        stats.bytes += directory_bytes(&path);
    }
    stats
}

/// `Some(message)` when a new download must be refused because the library is
/// at or above the configured quota. `quota == 0` means unlimited.
pub fn quota_block(used: u64, quota: u64) -> Option<String> {
    if quota == 0 || used < quota {
        return None;
    }
    Some(format!(
        "Library quota reached: {} used of {}. Delete partial downloads, remove a novel, or raise the limit in Settings.",
        human_bytes(used),
        human_bytes(quota)
    ))
}

/// Compact, base-1024 size for log lines and errors. The UI formats its own
/// numbers; this keeps backend messages readable.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn removable_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_dir())
        .unwrap_or(false)
}

/// Remove job work trees that no live job owns. Returns what was removed,
/// what was kept because a job is still running, and failures.
pub fn cleanup_work_dirs(root: &Path, busy_job_ids: &HashSet<String>) -> CleanupReport {
    let mut report = CleanupReport::default();
    let work_root = root.join(WORK_DIR);
    let Ok(entries) = std::fs::read_dir(&work_root) else {
        return report;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    paths.sort();

    for path in paths {
        if !removable_directory(&path) {
            continue;
        }
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        if busy_job_ids.contains(&name) {
            report.kept.push(name);
            continue;
        }
        let bytes = directory_bytes(&path);
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                report.bytes_freed += bytes;
                report.removed.push(name);
            }
            Err(error) => report.errors.push(format!("{name}: {error}")),
        }
    }

    // Drop the container when it is empty so a clean library has no hidden
    // tree at all.
    let _ = std::fs::remove_dir(&work_root);
    report
}

/// Delete every incomplete structured folder. Complete folders are skipped by
/// construction, and a folder that a job is writing is kept even when its
/// metadata still says incomplete.
pub fn delete_partial_folders(root: &Path, busy: &HashSet<PathBuf>) -> CleanupReport {
    let mut report = CleanupReport::default();
    let mut folders = Vec::new();

    for entry in WalkDir::new(root)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.file_name() == "metadata.json")
    {
        let Some(folder) = entry.path().parent() else {
            continue;
        };
        if paths::is_hidden_path(root, folder) {
            continue;
        }
        folders.push(folder.to_path_buf());
    }
    folders.sort();

    for folder in folders {
        let Some(metadata) = export::read_export_metadata(&folder) else {
            continue;
        };
        if export::export_is_complete(&metadata) {
            // A complete download is never part of this action.
            continue;
        }
        let name = folder
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| folder.to_string_lossy().to_string());
        let canonical = folder.canonicalize().unwrap_or_else(|_| folder.clone());
        if busy.contains(&canonical) {
            report.kept.push(name);
            continue;
        }
        let bytes = directory_bytes(&folder);
        match std::fs::remove_dir_all(&folder) {
            Ok(()) => {
                report.bytes_freed += bytes;
                report.removed.push(name);
            }
            Err(error) => report.errors.push(format!("{name}: {error}")),
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ExportMetadata;
    use std::io::Write;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "novelforge-storage-{label}-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(&vec![b'x'; bytes]).unwrap();
    }

    fn metadata_json(folder: &Path, complete: bool) {
        let metadata = ExportMetadata {
            title: "Fixture".to_string(),
            source_url: Some("https://example.com/book".to_string()),
            source: Some("example.com".to_string()),
            chapter_count: 2,
            downloaded_chapters: if complete { 2 } else { 1 },
            complete,
            index_complete: complete,
            selection: Some("all".to_string()),
            ..Default::default()
        };
        std::fs::create_dir_all(folder).unwrap();
        std::fs::write(
            folder.join("metadata.json"),
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn library_bytes_ignore_hidden_work_trees() {
        let root = temp_root("bytes");
        write(&root.join("book/metadata.json"), 100);
        write(&root.join("book/chapters/0001.xhtml"), 200);
        write(&root.join(".novelforge-work/job/novels/id/book.epub"), 400);
        write(&root.join(".partial/scratch"), 50);

        assert_eq!(library_bytes(&root), 300);
        assert_eq!(directory_bytes(&root.join("book")), 300);

        let work = work_stats(&root);
        assert_eq!(work.folders, 1);
        assert_eq!(work.bytes, 400);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn quota_only_blocks_at_or_above_the_limit() {
        assert!(quota_block(10, 0).is_none(), "zero means unlimited");
        assert!(quota_block(10, 20).is_none());
        assert!(quota_block(20, 20).is_some());
        assert!(quota_block(21, 20).is_some());
    }

    #[test]
    fn cleanup_work_dirs_keeps_running_jobs() {
        let root = temp_root("work");
        write(
            &root.join(".novelforge-work/done-job/artifacts/book.epub"),
            64,
        );
        write(
            &root.join(".novelforge-work/live-job/novels/id/book.epub"),
            32,
        );
        let busy = HashSet::from(["live-job".to_string()]);

        let report = cleanup_work_dirs(&root, &busy);
        assert_eq!(report.removed, vec!["done-job".to_string()]);
        assert_eq!(report.kept, vec!["live-job".to_string()]);
        assert_eq!(report.bytes_freed, 64);
        assert!(!root.join(".novelforge-work/done-job").exists());
        assert!(root.join(".novelforge-work/live-job").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn delete_partial_folders_never_touches_complete_or_busy() {
        let root = temp_root("partials");
        let complete = root.join("complete-book");
        let partial = root.join("partial-book");
        let busy_partial = root.join("busy-book");
        metadata_json(&complete, true);
        metadata_json(&partial, false);
        metadata_json(&busy_partial, false);
        write(&complete.join("chapters/0001.xhtml"), 10);
        write(&partial.join("chapters/0001.xhtml"), 20);
        write(&busy_partial.join("chapters/0001.xhtml"), 30);

        let busy = HashSet::from([busy_partial.canonicalize().unwrap()]);
        let report = delete_partial_folders(&root, &busy);

        assert_eq!(report.removed, vec!["partial-book".to_string()]);
        assert_eq!(report.kept, vec!["busy-book".to_string()]);
        assert!(report.bytes_freed >= 20);
        assert!(complete.exists(), "complete downloads stay put");
        assert!(!partial.exists());
        assert!(busy_partial.exists());

        let _ = std::fs::remove_dir_all(&root);
    }
}
