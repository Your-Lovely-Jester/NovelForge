//! First-class recovery actions for structured downloads.
//!
//! The library works on a concrete folder, not on a title or a "newest match"
//! heuristic. Every function re-validates that folder against the download root
//! before reading, updating or deleting it.

use crate::error::{AppError, AppResult};
use crate::export;
use crate::model::{
    DownloadDetails, DownloadOptions, DownloadRequest, DownloadTask, DuplicateMode, Engine,
    ExportMetadata, NovelChapterIndex,
};
use crate::paths;
use crate::process::{self, AppState};
use crate::settings::{Settings, now_ms, prepare_download_dir};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

fn current_settings(state: &AppState) -> Settings {
    state
        .settings
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

/// Resolve and validate a structured download folder. The folder must exist,
/// contain a readable `metadata.json`, and live under the download root.
pub fn read_download(root: &Path, folder: &str) -> AppResult<(PathBuf, ExportMetadata)> {
    let canonical_root = paths::canonical_root(root)?;
    let canonical = paths::contained_path(&canonical_root, Path::new(folder))?;
    if canonical == canonical_root {
        return Err(AppError::msg(
            "The download root itself is not a structured download",
        ));
    }
    if paths::is_hidden_path(&canonical_root, &canonical) {
        return Err(AppError::msg(
            "Hidden work folders cannot be opened as library downloads",
        ));
    }
    if !canonical.is_dir() {
        return Err(AppError::msg("The selected download is not a folder"));
    }
    let metadata = export::read_export_metadata(&canonical).ok_or_else(|| {
        AppError::msg(
            "This folder has no readable metadata.json; it is not a structured NovelForge download.",
        )
    })?;
    Ok((canonical, metadata))
}

pub fn details(root: &Path, folder: &str) -> AppResult<DownloadDetails> {
    let (canonical, metadata) = read_download(root, folder)?;
    let complete = export::export_is_complete(&metadata);
    Ok(DownloadDetails {
        folder: canonical.to_string_lossy().to_string(),
        title: metadata.title,
        source_url: metadata.source_url,
        chapter_count: metadata.chapter_count,
        downloaded_chapters: metadata.downloaded_chapters,
        selection: metadata.selection,
        index_complete: metadata.index_complete,
        index_detail: metadata.index_detail,
        complete,
        chapters: metadata.chapters,
    })
}

/// Resolve the chapters folder for the "Open chapters" action. Falls back to
/// the structured folder when an older export has no `chapters` directory.
pub fn chapter_folder(root: &Path, folder: &str) -> AppResult<PathBuf> {
    let (canonical, _) = read_download(root, folder)?;
    let chapters = canonical.join("chapters");
    Ok(if chapters.is_dir() {
        chapters
    } else {
        canonical
    })
}

pub fn delete_download(app: &AppHandle, state: &AppState, folder: String) -> AppResult<()> {
    let settings = current_settings(state);
    let root = prepare_download_dir(&settings)?;
    let canonical_root = paths::canonical_root(&root)?;
    let canonical = paths::contained_path(&canonical_root, Path::new(&folder))?;
    if canonical == canonical_root {
        return Err(AppError::msg("Refusing to delete the download root"));
    }
    if paths::is_hidden_path(&canonical_root, &canonical) {
        return Err(AppError::msg(
            "Hidden work folders cannot be deleted through the library",
        ));
    }
    if !canonical.is_dir() || !canonical.join("metadata.json").is_file() {
        return Err(AppError::msg(
            "Refusing to delete a folder that is not a structured NovelForge download",
        ));
    }
    if job_targets_folder(state, &canonical) {
        return Err(AppError::msg(
            "This folder is still being written by a running download",
        ));
    }

    std::fs::remove_dir_all(&canonical)?;
    let _ = app.emit(crate::model::EVENT_LIBRARY, now_ms());
    Ok(())
}

fn job_targets_folder(state: &AppState, folder: &Path) -> bool {
    state
        .jobs
        .lock()
        .map(|jobs| {
            jobs.values().any(|record| {
                !record.task.status.is_terminal()
                    && Path::new(&record.task.destination)
                        .canonicalize()
                        .ok()
                        .as_deref()
                        == Some(folder)
            })
        })
        .unwrap_or(false)
}

/// Canonical folders that a live job is currently writing. Bulk cleanup uses
/// this set so it can never delete a folder mid-download.
pub fn active_job_folders(state: &AppState) -> std::collections::HashSet<PathBuf> {
    state
        .jobs
        .lock()
        .map(|jobs| {
            jobs.values()
                .filter(|record| !record.task.status.is_terminal())
                .filter_map(|record| Path::new(&record.task.destination).canonicalize().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub async fn retry_download(
    app: AppHandle,
    state: AppState,
    folder: String,
    refresh: bool,
) -> AppResult<DownloadTask> {
    let settings = current_settings(&state);
    let root = prepare_download_dir(&settings)?;
    let (canonical, metadata) = read_download(&root, &folder)?;
    let source_url = source_url_for(&metadata)?;

    let chapters = if refresh {
        let index = crate::metadata::fetch_chapter_index_for(&settings, &source_url, true).await?;
        update_chapter_spec(&metadata, &canonical, &index, true)?
    } else {
        match retry_chapter_spec(&metadata, &canonical) {
            Some(spec) => spec,
            None => {
                // The previous manifest may not know which chapters are
                // missing (for example an older export without a full index).
                // Fetch the current index before deciding there is nothing to do.
                let index =
                    crate::metadata::fetch_chapter_index_for(&settings, &source_url, true).await?;
                update_chapter_spec(&metadata, &canonical, &index, false)?
            }
        }
    };

    let expected = export::parse_selection_serials(&chapters)
        .map(|values| values.len() as u32)
        .unwrap_or(metadata.chapter_count);
    start_update(
        app, state, canonical, metadata, source_url, chapters, expected,
    )
    .await
}

pub async fn update_download(
    app: AppHandle,
    state: AppState,
    folder: String,
    refresh: bool,
) -> AppResult<DownloadTask> {
    let settings = current_settings(&state);
    let root = prepare_download_dir(&settings)?;
    let (canonical, metadata) = read_download(&root, &folder)?;
    let source_url = source_url_for(&metadata)?;
    let index = crate::metadata::fetch_chapter_index_for(&settings, &source_url, true).await?;
    let chapters = update_chapter_spec(&metadata, &canonical, &index, refresh)?;
    let expected = export::parse_selection_serials(&chapters)
        .map(|values| values.len() as u32)
        .unwrap_or_else(|| index.total.max(index.chapters.len() as u32));
    start_update(
        app, state, canonical, metadata, source_url, chapters, expected,
    )
    .await
}

/// A verified one-hop candidate for a stored download, plus why it is offered.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FasterSource {
    pub title: String,
    pub url: String,
    pub host: String,
    pub detail: String,
    pub confidence: f32,
    pub current_host: Option<String>,
    pub current_rank: u8,
    pub candidate_rank: u8,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FasterSourceReport {
    /// Present only when a measured-cheaper host verified the same title.
    pub alternative: Option<FasterSource>,
    /// Always populated: why an alternative was or was not offered.
    pub note: String,
    pub current_host: Option<String>,
    pub current_rank: u8,
}

/// NF-041: look for the same title on a measured-cheaper host. This never
/// switches anything by itself; the caller offers a new download and the stored
/// entry keeps its `sourceUrl`.
pub async fn suggest_faster_source(
    state: AppState,
    folder: String,
) -> AppResult<FasterSourceReport> {
    let settings = current_settings(&state);
    let root = prepare_download_dir(&settings)?;
    let (_canonical, metadata) = read_download(&root, &folder)?;
    let source_url = source_url_for(&metadata)?;
    let current_host = url::Url::parse(&source_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string));
    let current_rank = current_host
        .as_deref()
        .map(crate::sources::rank_for_host)
        .unwrap_or(u8::MAX);

    if current_rank == 0 {
        return Ok(FasterSourceReport {
            alternative: None,
            note: format!(
                "{} is already the fastest measured host. Its chapters are one request each; no measured-cheaper source exists.",
                current_host
                    .clone()
                    .unwrap_or_else(|| "This source".to_string())
            ),
            current_host,
            current_rank,
        });
    }

    let resolved =
        crate::resolver::resolve_request(&settings, Engine::Webnovel, &metadata.title).await?;

    let mut candidates: Vec<FasterSource> = Vec::new();
    let mut consider = |title: &str, url: &str, detail: &str, confidence: f32| {
        let Ok(parsed) = url::Url::parse(url) else {
            return;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            return;
        }
        let Some(host) = parsed.host_str().map(str::to_string) else {
            return;
        };
        if Some(host.as_str()) == current_host.as_deref() {
            return;
        }
        let rank = crate::sources::rank_for_host(&host);
        if rank >= current_rank {
            return;
        }
        let profile = crate::sources::for_host(&host);
        if !profile.support.is_download_ready() {
            return;
        }
        // Never offer a lookalike. The resolver already verified this row, but
        // a different edition with a near title must still fail closed.
        if crate::resolver::relevance(&metadata.title, title) < 0.85 {
            return;
        }
        if candidates.iter().any(|candidate| {
            crate::export::normalize_source_url(&candidate.url)
                == crate::export::normalize_source_url(url)
        }) {
            return;
        }
        candidates.push(FasterSource {
            title: title.to_string(),
            url: url.to_string(),
            host,
            detail: detail.to_string(),
            confidence,
            current_host: current_host.clone(),
            current_rank,
            candidate_rank: rank,
        });
    };

    consider(
        &resolved.title,
        &resolved.source,
        &resolved.detail,
        resolved.confidence,
    );
    for alternative in &resolved.alternatives {
        consider(
            &alternative.title,
            &alternative.url,
            alternative
                .detail
                .as_deref()
                .unwrap_or("resolver alternative"),
            0.0,
        );
    }
    candidates.sort_by(|left, right| {
        left.candidate_rank
            .cmp(&right.candidate_rank)
            .then_with(|| right.confidence.total_cmp(&left.confidence))
    });

    let alternative = candidates.into_iter().next();
    let note = match (&alternative, current_host.as_deref()) {
        (Some(candidate), Some(host)) => format!(
            "Verified “{}” on {} — measured cheaper than {host}. Starting it creates a new library folder; the existing entry at {host} is not changed.",
            candidate.title, candidate.host
        ),
        (Some(candidate), None) => format!(
            "Verified “{}” on {} — measured cheaper than the current unnamed source.",
            candidate.title, candidate.host
        ),
        (None, Some(host)) => format!(
            "No measured-cheaper host verified “{}”. Keeping {host}; the update still uses the stored source.",
            metadata.title
        ),
        (None, None) => format!(
            "No measured-cheaper host verified “{}”. The stored source stays in use.",
            metadata.title
        ),
    };
    Ok(FasterSourceReport {
        alternative,
        note,
        current_host,
        current_rank,
    })
}

fn source_url_for(metadata: &ExportMetadata) -> AppResult<String> {
    if !export::is_webnovel_export(metadata) {
        return Err(AppError::msg(
            "Only structured webnovel downloads can be retried or updated from the library.",
        ));
    }
    metadata
        .source_url
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::msg("This download has no source URL to retry"))
}

async fn start_update(
    app: AppHandle,
    state: AppState,
    folder: PathBuf,
    metadata: ExportMetadata,
    source_url: String,
    chapters: String,
    expected_chapters: u32,
) -> AppResult<DownloadTask> {
    let pacing = metadata.crawl.clone();
    let host = metadata
        .source_url
        .as_deref()
        .and_then(|value| url::Url::parse(value).ok())
        .and_then(|url| url.host_str().map(str::to_string));
    let request = DownloadRequest {
        engine: Engine::Webnovel,
        query: source_url,
        title: Some(metadata.title),
        options: DownloadOptions {
            chapters: Some(chapters),
            jobs: None,
            throttle: Some(true),
            book_source: None,
            book_title: None,
            book_author: None,
            book_isbn: None,
            book_series: None,
            book_series_position: None,
            duplicate: Some(DuplicateMode::Update),
            target_folder: Some(folder.to_string_lossy().to_string()),
        },
    };
    let mut task = process::start_download(app.clone(), state.clone(), request).await?;
    // NF-041: an update carries the pace measured by the folder's last crawl,
    // so the job row can show what this source is expected to do instead of a
    // bar that looks stuck. Nothing is shown when no run has measured it.
    if let Some(pace) = pacing.filter(|pace| pace.chapters_per_minute > 0.0) {
        let expected_seconds =
            process::expected_seconds_for(expected_chapters, pace.chapters_per_minute);
        if let Some(updated) = crate::process::annotate_expected_pace(
            &app,
            &state,
            &task.id,
            pace.chapters_per_minute,
            expected_seconds,
        ) {
            task = updated;
        }
        crate::process::log_expected_pace(
            &app,
            &task.id,
            host.as_deref().unwrap_or("this source"),
            pace.chapters_per_minute,
            expected_chapters,
            expected_seconds,
        );
    }
    Ok(task)
}

fn retry_chapter_spec(metadata: &ExportMetadata, folder: &Path) -> Option<String> {
    let downloaded = downloaded_numbers(metadata, folder);
    let selected = metadata
        .selection
        .as_deref()
        .and_then(export::parse_selection_serials);
    let candidates = match selected {
        Some(values) => values,
        None => {
            // NF-044: the manifest's own numbers are the numbers the source
            // shows. On a catalogue that starts at Chapter 0 there is no
            // Chapter `chapter_count`, so `1..=total` is not the same set.
            let numbers = metadata
                .chapters
                .iter()
                .map(|chapter| chapter.number)
                .collect::<Vec<_>>();
            if !numbers.is_empty() && numbers.len() as u32 >= metadata.chapter_count {
                numbers
            } else {
                let total = metadata.chapter_count.max(metadata.chapters.len() as u32);
                if total == 0 {
                    return None;
                }
                (1..=total).collect()
            }
        }
    };
    let missing = candidates
        .into_iter()
        .filter(|number| !downloaded.contains(number))
        .collect::<Vec<_>>();
    (!missing.is_empty()).then(|| compress_numbers(&missing))
}

fn update_chapter_spec(
    metadata: &ExportMetadata,
    folder: &Path,
    index: &NovelChapterIndex,
    refresh: bool,
) -> AppResult<String> {
    if !index.complete {
        let detail = index
            .detail
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!(" {value}"))
            .unwrap_or_default();
        return Err(AppError::msg(format!(
            "The source index is incomplete, so the update was not started.{detail}"
        )));
    }
    let total = index.total.max(index.chapters.len() as u32);
    if total == 0 {
        return Err(AppError::msg("The source index is empty"));
    }
    if refresh {
        return full_chapter_spec(index);
    }

    let downloaded = downloaded_numbers(metadata, folder);
    let mut targets = index
        .chapters
        .iter()
        .map(|chapter| chapter.number)
        .filter(|number| !downloaded.contains(number))
        .collect::<Vec<_>>();
    targets.sort_unstable();
    targets.dedup();
    if targets.is_empty() {
        return Err(AppError::msg(
            "Already up to date — every chapter in the current index is downloaded. Use Refresh all chapters to re-download them.",
        ));
    }
    Ok(compress_numbers(&targets))
}

fn downloaded_numbers(metadata: &ExportMetadata, folder: &Path) -> HashSet<u32> {
    metadata
        .chapters
        .iter()
        .filter(|chapter| {
            chapter.downloaded
                && chapter
                    .file
                    .as_deref()
                    .is_some_and(|file| !file.contains("..") && folder.join(file).exists())
        })
        .map(|chapter| chapter.number)
        .collect()
}

fn full_chapter_spec(index: &NovelChapterIndex) -> AppResult<String> {
    // NF-044: refresh every row the index lists. `1..=total` is wrong for a
    // catalogue that starts at Chapter 0 or has a number missing from the TOC
    // (ORV shows 0-551 with 111 absent).
    let mut numbers = index
        .chapters
        .iter()
        .map(|chapter| chapter.number)
        .collect::<Vec<_>>();
    numbers.sort_unstable();
    numbers.dedup();
    if numbers.is_empty() {
        return Err(AppError::msg("The source index is empty"));
    }
    Ok(compress_numbers(&numbers))
}

fn compress_numbers(numbers: &[u32]) -> String {
    if numbers.is_empty() {
        return String::new();
    }
    let mut sorted = numbers.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut parts = Vec::new();
    let mut start = sorted[0];
    let mut end = start;
    for &number in &sorted[1..] {
        if number == end.saturating_add(1) {
            end = number;
        } else {
            parts.push(range_label(start, end));
            start = number;
            end = number;
        }
    }
    parts.push(range_label(start, end));
    parts.join(",")
}

fn range_label(start: u32, end: u32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ExportVerification, NovelChapter};

    fn chapter(number: u32, downloaded: bool) -> NovelChapter {
        NovelChapter {
            number,
            title: format!("Chapter {number}"),
            url: "https://example.com/book".to_string(),
            volume: None,
            downloaded,
            file: downloaded.then(|| format!("chapters/{number:04} - Chapter {number}.xhtml")),
        }
    }

    fn fixture_metadata(chapters: Vec<NovelChapter>, chapter_count: u32) -> ExportMetadata {
        ExportMetadata {
            title: "Fixture".to_string(),
            source_url: Some("https://example.com/book".to_string()),
            source: Some("example.com".to_string()),
            epub: Some("Fixture.epub".to_string()),
            chapter_count,
            downloaded_chapters: chapters.iter().filter(|c| c.downloaded).count() as u32,
            complete: false,
            chapters,
            verified: ExportVerification::default(),
            ..Default::default()
        }
    }

    fn fixture_folder(label: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "novelforge-actions-{label}-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(folder.join("chapters")).unwrap();
        folder
    }

    fn write_chapter(folder: &Path, number: u32) {
        std::fs::write(
            folder.join(format!("chapters/{number:04} - Chapter {number}.xhtml")),
            "<html>chapter</html>",
        )
        .unwrap();
    }

    #[test]
    fn compresses_missing_chapters_and_ignores_files_that_disappeared() {
        let folder = fixture_folder("retry");
        for number in [1, 2, 4] {
            write_chapter(&folder, number);
        }
        let metadata = fixture_metadata(
            vec![
                chapter(1, true),
                chapter(2, true),
                chapter(3, false),
                chapter(4, true),
                chapter(5, false),
            ],
            5,
        );
        assert_eq!(
            retry_chapter_spec(&metadata, &folder).as_deref(),
            Some("3,5")
        );

        // A manifest entry is not enough: the file must still exist.
        std::fs::remove_file(folder.join("chapters/0002 - Chapter 2.xhtml")).unwrap();
        assert_eq!(
            retry_chapter_spec(&metadata, &folder).as_deref(),
            Some("2-3,5")
        );

        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn retry_respects_the_stored_selection() {
        let folder = fixture_folder("retry-selection");
        for number in [1, 2, 4] {
            write_chapter(&folder, number);
        }
        let mut metadata = fixture_metadata(
            vec![
                chapter(1, true),
                chapter(2, true),
                chapter(3, false),
                chapter(4, true),
                chapter(5, false),
            ],
            5,
        );
        metadata.selection = Some("4-5".to_string());
        assert_eq!(retry_chapter_spec(&metadata, &folder).as_deref(), Some("5"));

        write_chapter(&folder, 5);
        metadata.chapters[4].downloaded = true;
        metadata.chapters[4].file = Some("chapters/0005 - Chapter 5.xhtml".to_string());
        assert_eq!(retry_chapter_spec(&metadata, &folder), None);

        // "all" is not a concrete selection; fall back to the full manifest.
        metadata.selection = Some("all".to_string());
        assert_eq!(retry_chapter_spec(&metadata, &folder).as_deref(), Some("3"));

        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn update_skips_existing_chapters_and_adds_new_ones() {
        let folder = fixture_folder("update");
        for number in [1, 2, 3] {
            write_chapter(&folder, number);
        }
        let metadata = fixture_metadata(
            vec![chapter(1, true), chapter(2, true), chapter(3, true)],
            3,
        );
        let index = NovelChapterIndex {
            source_url: "https://example.com/book".to_string(),
            source: "example.com".to_string(),
            title: Some("Fixture".to_string()),
            total: 5,
            complete: true,
            detail: Some("last page reached".to_string()),
            chapters: (1..=5).map(|number| chapter(number, false)).collect(),
            cached: false,
            cache_age_seconds: None,
            transport: None,
            title_source: None,
            title_lookup_attempted: false,
            title_recovery_attempted: false,
        };

        assert_eq!(
            update_chapter_spec(&metadata, &folder, &index, false).unwrap(),
            "4-5"
        );
        assert_eq!(
            update_chapter_spec(&metadata, &folder, &index, true).unwrap(),
            "1-5"
        );
        // A stale manifest must not inflate a refresh past the current index.
        let stale = ExportMetadata {
            chapter_count: 9,
            ..metadata.clone()
        };
        assert_eq!(
            update_chapter_spec(&stale, &folder, &index, true).unwrap(),
            "1-5"
        );

        let complete_index = NovelChapterIndex {
            total: 3,
            chapters: (1..=3).map(|number| chapter(number, false)).collect(),
            ..index.clone()
        };
        assert!(update_chapter_spec(&metadata, &folder, &complete_index, false).is_err());

        let incomplete = NovelChapterIndex {
            complete: false,
            detail: Some("only 2 of 5 pages reached".to_string()),
            ..index
        };
        let error = update_chapter_spec(&metadata, &folder, &incomplete, false).unwrap_err();
        assert!(error.to_string().contains("incomplete"));

        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn full_spec_covers_every_listed_chapter() {
        let index = |numbers: &[u32]| NovelChapterIndex {
            source_url: "https://example.com/book".to_string(),
            source: "example.com".to_string(),
            title: Some("Fixture".to_string()),
            total: numbers.len() as u32,
            complete: true,
            detail: Some("fixture".to_string()),
            chapters: numbers
                .iter()
                .map(|number| chapter(*number, false))
                .collect(),
            cached: false,
            cache_age_seconds: None,
            transport: None,
            title_source: None,
            title_lookup_attempted: false,
            title_recovery_attempted: false,
        };

        assert_eq!(
            full_chapter_spec(&index(&(1..=113).collect::<Vec<_>>())).unwrap(),
            "1-113"
        );
        // ORV: the catalogue starts at 0 and skips a number the TOC never lists.
        assert_eq!(
            full_chapter_spec(&index(&[0, 1, 2, 3, 5])).unwrap(),
            "0-3,5"
        );
        assert!(full_chapter_spec(&index(&[])).is_err());
        assert_eq!(compress_numbers(&[1, 2, 3, 5, 7, 8]), "1-3,5,7-8");
    }

    #[test]
    fn read_download_rejects_folders_without_metadata() {
        let root = fixture_folder("read-root");
        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(read_download(&root, &plain.to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
