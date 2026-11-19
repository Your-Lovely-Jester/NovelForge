//! Detect an existing structured download for the same source URL.
//!
//! Source URLs are compared by a canonical identity rather than by title or
//! folder name. This is what stops a repeat download from silently creating a
//! second `novel-<uuid>` folder.

use crate::export;
use crate::model::ExistingDownload;
use crate::paths::is_hidden_path;
use std::path::Path;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub fn find_existing_download(root: &Path, source_url: &str) -> Option<ExistingDownload> {
    let target = export::normalize_source_url(source_url);
    if target.is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    for entry in WalkDir::new(root)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| !is_hidden_path(root, entry.path()))
        .filter(|entry| entry.file_name() == "metadata.json")
    {
        let Some(folder) = entry.path().parent() else {
            continue;
        };
        let Some(metadata) = export::read_export_metadata(folder) else {
            continue;
        };
        let Some(metadata_url) = metadata.source_url.as_deref() else {
            continue;
        };
        if export::normalize_source_url(metadata_url) != target {
            continue;
        }
        // Keep sub-second precision for ordering. `modified` is a display value
        // in whole seconds, but two downloads can finish in the same second
        // (and Windows file times are coarser than Linux's), so sorting on the
        // truncated field picked the older folder.
        let modified_at = entry
            .metadata()
            .ok()
            .and_then(|value| value.modified().ok())
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        let modified = (modified_at / 1_000_000_000) as u64;
        let epub_path = metadata.epub.as_ref().and_then(|name| {
            let path = folder.join(name);
            path.is_file().then(|| path.to_string_lossy().to_string())
        });
        let complete = export::export_is_complete(&metadata);
        matches.push((
            modified_at,
            ExistingDownload {
                folder: folder.to_string_lossy().to_string(),
                title: metadata.title,
                source_url: metadata_url.to_string(),
                chapter_count: metadata.chapter_count,
                downloaded_chapters: metadata.downloaded_chapters,
                complete,
                modified,
                epub_path,
                matches: 1,
            },
        ));
    }

    matches.sort_by(|(left_at, left), (right_at, right)| {
        right
            .complete
            .cmp(&left.complete)
            .then_with(|| right_at.cmp(left_at))
    });
    let total = matches.len() as u32;
    matches.into_iter().next().map(|(_, mut existing)| {
        existing.matches = total;
        existing
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ExportMetadata, ExportVerification, NovelChapter};
    use std::path::PathBuf;

    fn fixture_folder(root: &Path, name: &str, source: &str, chapter_count: u32) -> PathBuf {
        let folder = root.join(name);
        std::fs::create_dir_all(folder.join("chapters")).unwrap();
        let chapter_file = "chapters/0001 - Chapter 1.xhtml";
        std::fs::write(folder.join(chapter_file), "<html>chapter</html>").unwrap();
        let metadata = ExportMetadata {
            title: "Mother Of Learning".to_string(),
            source_url: Some(source.to_string()),
            source: Some("novelphoenix.net".to_string()),
            epub: Some("Mother Of Learning.epub".to_string()),
            chapter_count,
            downloaded_chapters: 1,
            complete: true,
            chapters: vec![NovelChapter {
                number: 1,
                title: "Chapter 1".to_string(),
                url: source.to_string(),
                volume: None,
                downloaded: true,
                file: Some(chapter_file.to_string()),
            }],
            verified: ExportVerification {
                ok: true,
                ..Default::default()
            },
            ..Default::default()
        };
        std::fs::write(
            folder.join("metadata.json"),
            serde_json::to_string_pretty(&metadata).unwrap(),
        )
        .unwrap();
        folder
    }

    #[test]
    fn normalizes_tracking_queries_and_trailing_slashes() {
        let base = "https://novelphoenix.net/mother-of-learning.html";
        assert_eq!(
            export::normalize_source_url(
                "http://www.novelphoenix.net/mother-of-learning.html/?utm_source=x&utm_medium=y#chapters"
            ),
            base
        );
        assert_eq!(
            export::normalize_source_url("https://novelphoenix.net/mother-of-learning.html/"),
            base
        );
        assert_eq!(
            export::normalize_source_url(
                "https://novelphoenix.net/mother-of-learning.html?fbclid=abc"
            ),
            base
        );
        // A meaningful query must not be discarded.
        assert_ne!(
            export::normalize_source_url("https://example.com/book?p=123"),
            export::normalize_source_url("https://example.com/book")
        );
    }

    #[test]
    fn finds_the_newest_complete_match_and_counts_duplicates() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-duplicates-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let older = fixture_folder(
            &root,
            "mother-of-learning-aaaaaaaa",
            "https://novelphoenix.net/mother-of-learning.html",
            113,
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        let newer = fixture_folder(
            &root,
            "mother-of-learning-bbbbbbbb",
            "https://novelphoenix.net/mother-of-learning.html/?utm_campaign=repeat",
            113,
        );
        // A different novel must not match.
        fixture_folder(
            &root,
            "other-book",
            "https://novelphoenix.net/other-book.html",
            10,
        );

        let existing =
            find_existing_download(&root, "https://novelphoenix.net/mother-of-learning.html")
                .expect("existing download");
        assert_eq!(existing.matches, 2);
        assert_eq!(existing.folder, newer.to_string_lossy());
        assert_ne!(existing.folder, older.to_string_lossy());
        assert!(existing.complete);

        let _ = std::fs::remove_dir_all(&root);
    }
}
