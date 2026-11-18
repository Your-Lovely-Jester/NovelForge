use crate::books;
use crate::catalogue;
use crate::duplicates;
use crate::error::{AppError, AppResult};
use crate::export;
use crate::library_actions;
use crate::metadata;
use crate::model::{
    AppInfo, CleanupReport, DownloadDetails, DownloadRequest, DownloadTask, Engine,
    ExistingDownload, LibraryArchive, LibraryEntry, LibraryNovel, LibraryResponse, LibraryStats,
    LibraryStorage, MangaSources, NovelChapterIndex, NovelMetadata, ResolvedTitle, SearchResponse,
    VerificationReport,
};
use crate::paths;
use crate::process::{self, AppState};
use crate::resolver;
use crate::settings::{Settings, now_ms, prepare_download_dir};
use crate::storage;
use crate::tools::{ToolStatus, ToolsReport, check_tools as inspect_tools};
use crate::transport::{self, TransportReport};
use crate::verify;
use std::collections::BTreeMap;
use std::path::Path;
use tauri::{AppHandle, Emitter, State};
use walkdir::WalkDir;

/// With `settings`, probe the values currently typed in Settings without
/// writing them to disk. Without it, probe the saved configuration.
#[tauri::command]
pub async fn check_tools(
    state: State<'_, AppState>,
    settings: Option<Settings>,
) -> AppResult<ToolsReport> {
    let effective = match settings {
        Some(preview) => preview.normalized(),
        None => state
            .settings
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default(),
    };
    Ok(inspect_tools(&effective).await)
}

/// Test one tool path immediately, so the Settings fields can answer
/// "found / not found" before the user saves anything.
#[tauri::command]
pub async fn probe_tool(id: String, path: String) -> AppResult<ToolStatus> {
    crate::tools::probe_tool_path(&id, &path)
        .await
        .ok_or_else(|| AppError::msg(format!("Unknown tool id: {id}")))
}

fn current_settings(state: &State<'_, AppState>) -> Settings {
    state
        .settings
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn resolve_title(
    state: State<'_, AppState>,
    engine: Engine,
    query: String,
    mode: Option<String>,
    sources: Option<Vec<String>>,
) -> AppResult<ResolvedTitle> {
    let mode = resolver::ResolveMode::parse(mode.as_deref());
    let selected = sources.unwrap_or_default();
    resolver::resolve_request_from(&current_settings(&state), engine, &query, mode, &selected).await
}

#[tauri::command]
pub async fn search_webnovels(
    state: State<'_, AppState>,
    query: String,
    sources: Option<Vec<String>>,
) -> AppResult<SearchResponse> {
    let selected = sources.unwrap_or_default();
    Ok(resolver::search_novels_from(&current_settings(&state), &query, &selected).await)
}

#[tauri::command]
pub async fn search_books(
    state: State<'_, AppState>,
    query: String,
    mode: Option<String>,
) -> AppResult<books::BookSearchResponse> {
    let settings = current_settings(&state);
    books::search(
        &settings,
        state.config_file.as_path(),
        &query,
        mode.as_deref().unwrap_or("auto"),
    )
    .await
}

#[tauri::command]
pub fn get_hardcover_status(state: State<'_, AppState>) -> AppResult<books::HardcoverStatus> {
    Ok(books::status(state.config_file.as_path()))
}

#[tauri::command]
pub fn set_hardcover_token(
    state: State<'_, AppState>,
    token: String,
) -> AppResult<books::HardcoverStatus> {
    books::write_token(state.config_file.as_path(), &token)
}

#[tauri::command]
pub fn clear_hardcover_token(state: State<'_, AppState>) -> AppResult<books::HardcoverStatus> {
    books::clear_token(state.config_file.as_path())
}

#[tauri::command]
pub fn list_source_catalogue() -> Vec<catalogue::CatalogueSource> {
    catalogue::all()
}

#[tauri::command]
pub async fn fetch_novel_metadata(url: String) -> AppResult<NovelMetadata> {
    metadata::fetch_novel_metadata(&url).await
}

#[tauri::command]
pub async fn fetch_novel_chapters(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    refresh: Option<bool>,
) -> AppResult<NovelChapterIndex> {
    let settings = current_settings(&state);
    // A 3,000-chapter book needs about a minute of TOC pages at the safe
    // request gap. Emit the page counter so the picker can show the walk
    // moving instead of a static spinner.
    let event_url = url.clone();
    let report = move |page: u32, pages: u32| {
        let _ = app.emit(
            crate::model::EVENT_INDEX_PROGRESS,
            serde_json::json!({ "url": event_url, "page": page, "pages": pages }),
        );
    };
    metadata::fetch_chapter_index_reporting(&settings, &url, refresh.unwrap_or(false), &report)
        .await
}

#[tauri::command]
pub fn find_existing_download(
    state: State<'_, AppState>,
    url: String,
) -> AppResult<Option<ExistingDownload>> {
    let settings = current_settings(&state);
    let root = prepare_download_dir(&settings)?;
    Ok(duplicates::find_existing_download(&root, &url))
}

#[tauri::command]
pub fn get_download_details(
    state: State<'_, AppState>,
    folder: String,
) -> AppResult<DownloadDetails> {
    let settings = current_settings(&state);
    let root = prepare_download_dir(&settings)?;
    library_actions::details(&root, &folder)
}

#[tauri::command]
pub fn delete_download(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: String,
) -> AppResult<()> {
    library_actions::delete_download(&app, state.inner(), folder)
}

#[tauri::command]
pub async fn retry_download(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: String,
    refresh: Option<bool>,
) -> AppResult<DownloadTask> {
    library_actions::retry_download(app, state.inner().clone(), folder, refresh.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn update_download(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: String,
    refresh: Option<bool>,
) -> AppResult<DownloadTask> {
    library_actions::update_download(app, state.inner().clone(), folder, refresh.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn suggest_faster_source(
    state: State<'_, AppState>,
    folder: String,
) -> AppResult<library_actions::FasterSourceReport> {
    library_actions::suggest_faster_source(state.inner().clone(), folder).await
}

#[tauri::command]
pub fn open_chapters(state: State<'_, AppState>, folder: String) -> AppResult<()> {
    let settings = current_settings(&state);
    let root = prepare_download_dir(&settings)?;
    let target = library_actions::chapter_folder(&root, &folder)?;
    open_in_file_manager(&target)
}

#[tauri::command]
pub async fn list_manga_sources(state: State<'_, AppState>) -> AppResult<MangaSources> {
    resolver::list_manga_sources(&current_settings(&state)).await
}

#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    state: State<'_, AppState>,
    request: DownloadRequest,
) -> AppResult<DownloadTask> {
    process::start_download(app, state.inner().clone(), request).await
}

#[tauri::command]
pub fn pause_download(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> AppResult<DownloadTask> {
    process::pause_download(&app, &state, &job_id)
}

#[tauri::command]
pub fn resume_download(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> AppResult<DownloadTask> {
    process::resume_download(&app, &state, &job_id)
}

#[tauri::command]
pub fn cancel_download(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> AppResult<DownloadTask> {
    process::cancel_download(&app, &state, &job_id)
}

#[tauri::command]
pub fn clear_finished(state: State<'_, AppState>) -> Vec<DownloadTask> {
    process::clear_finished(&state)
}

#[tauri::command]
pub fn get_tasks(state: State<'_, AppState>) -> Vec<DownloadTask> {
    process::list_tasks(&state)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppResult<Settings> {
    state
        .settings
        .read()
        .map(|guard| guard.clone())
        .map_err(|_| AppError::msg("Settings store unavailable"))
}

#[tauri::command]
pub fn get_transport_report() -> TransportReport {
    transport::report()
}

/// Declarative source registry, including support level and evidence. The UI
/// uses this to show what is actually verified instead of implying every host
/// is fully supported.
#[tauri::command]
pub fn list_source_profiles() -> Vec<crate::sources::SourceSummary> {
    crate::sources::summaries()
}

#[tauri::command]
pub async fn save_settings(state: State<'_, AppState>, settings: Settings) -> AppResult<Settings> {
    let mut normalized = settings.normalized();
    validate_tool_path(normalized.lncrawl_path.as_deref(), "lncrawl")?;
    validate_tool_path(normalized.gallery_dl_path.as_deref(), "gallery-dl")?;
    validate_tool_path(normalized.aget_path.as_deref(), "aget")?;

    let root = prepare_download_dir(&normalized)?;
    normalized.download_dir = root.to_string_lossy().to_string();
    normalized.save(state.config_file.as_path())?;
    transport::configure(&normalized.source_limits);

    let mut guard = state
        .settings
        .write()
        .map_err(|_| AppError::msg("Settings store unavailable"))?;
    *guard = normalized.clone();
    Ok(normalized)
}

fn validate_tool_path(path: Option<&str>, label: &str) -> AppResult<()> {
    if let Some(path) = path {
        let candidate = Path::new(path);
        if !candidate.is_file() {
            return Err(AppError::msg(format!(
                "{label} path does not point to a file: {}",
                candidate.display()
            )));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn list_library(state: State<'_, AppState>) -> AppResult<LibraryResponse> {
    let settings = state
        .settings
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let root = prepare_download_dir(&settings)?;
    let mut entries = Vec::new();

    for entry in WalkDir::new(&root)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| !paths::is_hidden_path(&root, entry.path()))
    {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let Some(kind) = library_kind(path) else {
            continue;
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let folder = path
            .parent()
            .and_then(Path::file_name)
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "NovelForge".to_string());

        entries.push(LibraryEntry {
            name,
            path: path.to_string_lossy().to_string(),
            size: metadata.len(),
            modified,
            kind,
            folder,
        });
    }

    entries.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| left.name.cmp(&right.name))
    });
    entries.truncate(500);

    let mut by_kind = BTreeMap::new();
    for entry in &entries {
        *by_kind.entry(entry.kind.clone()).or_insert(0_u64) += entry.size;
    }

    // Storage used should reflect the structured folders too: metadata.json,
    // covers, chapter files and manga images all consume local disk. It is
    // measured after the index backfill below so the number matches the
    // per-novel sizes on the same scan.

    let mut novels = Vec::new();
    for entry in WalkDir::new(&root)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| !paths::is_hidden_path(&root, entry.path()))
        .filter(|entry| entry.file_name() == "metadata.json")
    {
        if let Some(folder) = entry.path().parent()
            && let Some(mut novel) = export::library_novel(folder)
        {
            novel.disk_bytes = storage::directory_bytes(folder);
            novels.push(novel);
        }
    }
    novels.sort_by_key(|novel| std::cmp::Reverse(novel.modified));

    // Partial folders are counted before the display list is truncated: the
    // cleanup action must describe the whole library, not the first 120 cards.
    let partial_bytes = novels
        .iter()
        .filter(|novel| !novel.complete)
        .map(|novel| novel.disk_bytes)
        .sum();
    let partial_folders = novels.iter().filter(|novel| !novel.complete).count() as u32;
    novels.truncate(120);

    let archives = library_archives(&root);
    let work = storage::work_stats(&root);
    let quota_bytes = settings.max_library_bytes;
    let total_bytes = storage::library_bytes(&root);

    Ok(LibraryResponse {
        stats: LibraryStats {
            count: entries.len(),
            total_bytes,
            by_kind,
        },
        entries,
        novels,
        archives,
        storage: LibraryStorage {
            library_bytes: total_bytes,
            partial_bytes,
            partial_folders,
            work_bytes: work.bytes,
            work_folders: work.folders,
            quota_bytes,
            over_quota: quota_bytes > 0 && total_bytes >= quota_bytes,
        },
    })
}

/// Loose archive files that are not part of a structured download.
///
/// Older NovelForge folders kept only the EPUB, so they never appeared as
/// novels. Each file gets an explicit adoptability decision instead of being
/// hidden: one EPUB in its own folder, with no `metadata.json`, can be split
/// into structured sidecars without touching the original.
fn library_archives(root: &Path) -> Vec<LibraryArchive> {
    let mut archives = Vec::new();

    for entry in WalkDir::new(root)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| !paths::is_hidden_path(root, entry.path()))
    {
        let path = entry.path();
        let Some(kind) = library_kind(path) else {
            continue;
        };
        let Some(folder) = path.parent() else {
            continue;
        };
        // Structured downloads already render as novels, including their EPUB.
        if folder.join("metadata.json").is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let folder_name = folder
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "NovelForge".to_string());

        let (adoptable, reason) = archive_adoptability(root, path, folder, &kind);
        archives.push(LibraryArchive {
            name,
            path: path.to_string_lossy().to_string(),
            folder: folder_name,
            kind,
            size: metadata.len(),
            modified,
            adoptable,
            reason,
        });
    }

    archives.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| left.name.cmp(&right.name))
    });
    archives.truncate(120);
    archives
}

fn archive_adoptability(root: &Path, path: &Path, folder: &Path, kind: &str) -> (bool, String) {
    if folder == root {
        return (
            false,
            "Move this file into its own folder to adopt it as a novel.".to_string(),
        );
    }
    if kind != "epub" {
        return (
            false,
            format!(
                "{} files stay archive-only; only EPUB can be split into chapters.",
                kind.to_ascii_uppercase()
            ),
        );
    }
    let chapters = folder.join("chapters");
    if chapters.is_dir()
        && std::fs::read_dir(&chapters)
            .map(|entries| entries.filter_map(Result::ok).next().is_some())
            .unwrap_or(false)
    {
        return (
            false,
            "This folder already has chapter files; move them aside before adopting.".to_string(),
        );
    }
    let siblings = std::fs::read_dir(folder)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|file| file.is_file()))
        .filter(|entry| !paths::is_hidden_path(root, &entry.path()))
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("epub"))
        })
        .count();
    if siblings > 1 {
        return (
            false,
            format!("Folder has {siblings} EPUBs; keep one EPUB per folder to adopt."),
        );
    }
    match export::audit_epub(path) {
        Ok(audit) => {
            let plan = export::plan_audit(&audit, None, None, None);
            if plan.ok {
                (true, format!("Ready to adopt: {}.", plan.detail))
            } else {
                (false, format!("EPUB audit failed: {}", plan.detail))
            }
        }
        Err(error) => (false, format!("EPUB audit failed: {error}")),
    }
}

#[tauri::command]
pub fn adopt_archive(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<LibraryNovel> {
    let settings = current_settings(&state);
    let root = prepare_download_dir(&settings)?;
    let canonical_root = paths::canonical_root(&root)?;
    let canonical = paths::contained_path(&canonical_root, Path::new(&path))?;
    if !canonical.is_file() {
        return Err(AppError::msg("The selected archive is not a file"));
    }
    if paths::is_hidden_path(&canonical_root, &canonical) {
        return Err(AppError::msg(
            "Hidden work folders cannot be adopted into the library",
        ));
    }
    if !canonical
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("epub"))
    {
        return Err(AppError::msg(
            "Only EPUB archives can be adopted into the structured library",
        ));
    }
    let folder = canonical
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| AppError::msg("The selected archive has no parent folder"))?;
    if folder == canonical_root {
        return Err(AppError::msg(
            "Move the EPUB into its own folder before adopting it",
        ));
    }
    if paths::is_hidden_path(&canonical_root, &folder) {
        return Err(AppError::msg(
            "Hidden work folders cannot be adopted into the library",
        ));
    }
    if job_targets_folder(&state, &folder) {
        return Err(AppError::msg(
            "This folder is still being written by a running download",
        ));
    }

    export::adopt_epub(&canonical, &folder).map_err(AppError::msg)?;
    let mut novel = export::library_novel(&folder)
        .ok_or_else(|| AppError::msg("The adopted folder could not be read back"))?;
    novel.disk_bytes = storage::directory_bytes(&folder);
    let _ = app.emit(crate::model::EVENT_LIBRARY, now_ms());
    Ok(novel)
}

/// Remove every abandoned `.novelforge-work/<job id>` tree. A tree owned by a
/// running or paused job is kept and reported back.
#[tauri::command]
pub fn cleanup_engine_work(app: AppHandle, state: State<'_, AppState>) -> AppResult<CleanupReport> {
    let settings = current_settings(&state);
    let root = prepare_download_dir(&settings)?;
    let busy = process::active_job_ids(&state);
    let report = storage::cleanup_work_dirs(&root, &busy);
    let _ = app.emit(crate::model::EVENT_LIBRARY, now_ms());
    Ok(report)
}

/// Delete incomplete structured folders in one action. Complete downloads are
/// excluded unconditionally, and folders owned by a live job are kept.
#[tauri::command]
pub fn delete_partial_downloads(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<CleanupReport> {
    let settings = current_settings(&state);
    let root = prepare_download_dir(&settings)?;
    let busy = library_actions::active_job_folders(&state);
    let report = storage::delete_partial_folders(&root, &busy);
    let _ = app.emit(crate::model::EVENT_LIBRARY, now_ms());
    Ok(report)
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
                        .is_ok_and(|target| target.starts_with(folder))
            })
        })
        .unwrap_or(false)
}

/// Hidden trees such as `.novelforge-work` hold partial crawler state and must
/// never appear as library entries or count toward library bytes.
fn library_kind(path: &Path) -> Option<String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "epub" | "cbz" | "zip" | "pdf" | "mobi" | "azw3" | "txt"
    )
    .then_some(extension)
}

#[tauri::command]
pub fn verify_path(state: State<'_, AppState>, path: String) -> AppResult<VerificationReport> {
    let settings = state
        .settings
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let root = prepare_download_dir(&settings)?;
    let canonical = paths::contained_path(&root, Path::new(&path))?;
    if !canonical.is_file() {
        return Err(AppError::msg("Verification target is not a file"));
    }
    Ok(verify::verify_any(&canonical, true))
}

#[tauri::command]
pub fn reveal_path(state: State<'_, AppState>, path: String) -> AppResult<()> {
    let settings = state
        .settings
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let root = prepare_download_dir(&settings)?;
    let canonical = paths::contained_path(&root, Path::new(&path))?;
    let target = if canonical.is_dir() {
        canonical
    } else {
        canonical
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.clone())
    };
    open_in_file_manager(&target)
}

/// Opens one file with the system handler (a CBZ in the comic reader, an EPUB
/// in the ebook reader). `reveal_path` deliberately opens the folder instead,
/// which is useless for the packed manga archive the library card links to.
#[tauri::command]
pub fn open_file(state: State<'_, AppState>, path: String) -> AppResult<()> {
    let settings = state
        .settings
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let root = prepare_download_dir(&settings)?;
    let target = paths::contained_path(&root, Path::new(&path))?;
    if !target.is_file() {
        return Err(AppError::msg(format!("Not a file: {}", target.display())));
    }
    open_in_file_manager(&target)
}

/// Rebuilds the packed CBZ for a comic folder that only carries the expanded
/// `chapters/` tree. Older builds surfaced the image folders, so an existing
/// library can hold comics with no archive; repacking repairs them in place
/// instead of asking for another crawl.
#[tauri::command]
pub fn repack_manga(state: State<'_, AppState>, folder: String) -> AppResult<String> {
    let settings = current_settings(&state);
    let root = prepare_download_dir(&settings)?;
    repack_structured_comic(&root, &folder, &settings.manga_format)
}

/// The filesystem half of [`repack_manga`], separated so the refusals can be
/// tested without a Tauri state handle.
fn repack_structured_comic(root: &Path, folder: &str, format: &str) -> AppResult<String> {
    let target = paths::contained_path(root, Path::new(folder))?;
    if !target.is_dir() {
        return Err(AppError::msg(format!("Not a folder: {}", target.display())));
    }
    let metadata = export::read_export_metadata(&target).ok_or_else(|| {
        AppError::msg("This folder has no metadata.json, so it is not a structured download.")
    })?;
    // A novel folder already ships an EPUB and its `chapters/` entries are
    // XHTML files, not pages; repacking is only meaningful for comics.
    if metadata.epub.is_some() {
        return Err(AppError::msg(
            "This folder already has an EPUB; rebuilding a comic archive only applies to manga.",
        ));
    }
    let title = if metadata.title.trim().is_empty() {
        target
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "manga".to_string())
    } else {
        metadata.title.clone()
    };
    let path = export::repack_manga_chapters(&target, format, &title).map_err(AppError::msg)?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn open_downloads(state: State<'_, AppState>) -> AppResult<()> {
    let settings = state
        .settings
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let root = prepare_download_dir(&settings)?;
    open_in_file_manager(&root)
}

/// Resume the download an interrupted journal row points at. The stored
/// `destination` is passed back through the normal retry path so the same
/// verification, selection and fail-closed rules apply.
#[tauri::command]
pub async fn resume_interrupted(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
) -> AppResult<DownloadTask> {
    let folder = {
        let jobs = state
            .jobs
            .lock()
            .map_err(|_| AppError::msg("Job store unavailable"))?;
        let record = jobs
            .get(&job_id)
            .ok_or_else(|| AppError::msg("Unknown download job"))?;
        if record.task.status != crate::model::TaskStatus::Interrupted {
            return Err(AppError::msg(
                "Only an interrupted download can be resumed from the journal",
            ));
        }
        record.task.destination.clone()
    };
    let task = library_actions::retry_download(app, state.inner().clone(), folder, false).await?;
    process::discard_task(&state, &job_id);
    Ok(task)
}

fn build_commit() -> String {
    option_env!("NOVELFORGE_COMMIT")
        .map(str::trim)
        .filter(|sha| !sha.is_empty())
        .map(|sha| sha.chars().take(7).collect())
        .unwrap_or_else(|| "dev".to_string())
}

#[tauri::command]
pub fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> AppResult<AppInfo> {
    let download_dir = state
        .settings
        .read()
        .map(|guard| guard.download_dir.clone())
        .unwrap_or_default();

    Ok(AppInfo {
        name: "NovelForge".to_string(),
        version: app.package_info().version.to_string(),
        commit: build_commit(),
        tauri_version: tauri::VERSION.to_string(),
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        download_dir,
    })
}

/// Exit after the user has seen and accepted the interrupted-download warning.
#[tauri::command]
pub fn confirm_quit(app: AppHandle, state: State<'_, AppState>) {
    state
        .allow_exit
        .store(true, std::sync::atomic::Ordering::SeqCst);
    process::shutdown_all(&state);
    app.exit(0);
}

/// A redacted, copy-pasteable report for a GitHub issue. Paths under the home
/// directory are shortened to `~`; no token, no chapter text, no page content.
#[tauri::command]
pub async fn get_diagnostics(app: AppHandle, state: State<'_, AppState>) -> AppResult<String> {
    use std::fmt::Write as _;

    let settings = current_settings(&state);
    let tools = inspect_tools(&settings).await;
    let tasks = process::list_tasks(&state);
    let mut report = String::new();
    let _ = writeln!(report, "NovelForge diagnostics");
    let _ = writeln!(report, "version: {}", app.package_info().version);
    let _ = writeln!(report, "commit: {}", build_commit());
    let _ = writeln!(report, "tauri: {}", tauri::VERSION);
    let _ = writeln!(
        report,
        "platform: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(report, "generatedAt: {} ms", now_ms());
    let _ = writeln!(
        report,
        "downloadRoot: {}",
        redact_home(&settings.download_dir)
    );
    let _ = writeln!(
        report,
        "configFile: {}",
        redact_home(&state.config_file.display().to_string())
    );
    let _ = writeln!(
        report,
        "logFile: {}",
        redact_home(&state.log_file.display().to_string())
    );
    let _ = writeln!(report);
    let _ = writeln!(report, "[settings]");
    let _ = writeln!(report, "defaultEngine: {}", settings.default_engine.label());
    let _ = writeln!(report, "mangaFormat: {}", settings.manga_format);
    let _ = writeln!(report, "mangaThrottle: {}", settings.manga_throttle);
    let _ = writeln!(report, "maxLibraryBytes: {}", settings.max_library_bytes);
    let _ = writeln!(
        report,
        "cleanupWorkAfterImport: {}",
        settings.cleanup_work_after_import
    );
    let _ = writeln!(report, "sourceLimits: {}", settings.source_limits.len());
    let _ = writeln!(report);
    let _ = writeln!(report, "[tools]");
    let _ = writeln!(
        report,
        "binDir: {}",
        tools
            .bin_dir
            .as_deref()
            .map(redact_home)
            .unwrap_or_else(|| "unresolved".to_string())
    );
    for (label, configured) in [
        ("lncrawl", settings.lncrawl_path.as_deref()),
        ("gallery-dl", settings.gallery_dl_path.as_deref()),
        ("aget", settings.aget_path.as_deref()),
    ] {
        let _ = writeln!(
            report,
            "configured {label}: {}",
            configured
                .map(redact_home)
                .unwrap_or_else(|| "auto".to_string())
        );
    }
    for tool in [&tools.lncrawl, &tools.gallery_dl, &tools.aget] {
        let _ = writeln!(
            report,
            "{}: found={} version={} path={}",
            tool.label,
            tool.found,
            tool.version.as_deref().unwrap_or("—"),
            tool.path
                .as_deref()
                .map(redact_home)
                .unwrap_or_else(|| "—".to_string())
        );
        if let Some(detail) = tool.detail.as_deref() {
            let _ = writeln!(report, "  detail: {}", redact_home(detail));
        }
    }
    let _ = writeln!(report);
    let _ = writeln!(report, "[active jobs]");
    let active = tasks
        .iter()
        .filter(|task| !task.status.is_terminal())
        .collect::<Vec<_>>();
    if active.is_empty() {
        let _ = writeln!(report, "none");
    } else {
        for task in active {
            let _ = writeln!(
                report,
                "{:?} {:.0}% {} — {}",
                task.status,
                task.progress,
                task.query,
                task.message.as_deref().unwrap_or("")
            );
        }
    }
    let _ = writeln!(report);
    let _ = writeln!(report, "[recent tasks]");
    let restored = tasks
        .iter()
        .filter(|task| task.status == crate::model::TaskStatus::Interrupted)
        .count();
    let _ = writeln!(report, "total={} interrupted={}", tasks.len(), restored);
    let _ = writeln!(report);
    let _ = writeln!(report, "[log tail]");
    let tail = process::log_tail(state.log_file.as_ref(), 200);
    if tail.is_empty() {
        let _ = writeln!(report, "(no log yet)");
    } else {
        for line in tail {
            let _ = writeln!(report, "{}", redact_home(&line));
        }
    }
    Ok(report)
}

/// Shorten the user's home directory so pasted diagnostics do not leak a name.
fn redact_home(value: &str) -> String {
    let Some(home) = dirs::home_dir() else {
        return value.to_string();
    };
    let home = home.to_string_lossy();
    if home.is_empty() {
        value.to_string()
    } else {
        value.replace(home.as_ref(), "~")
    }
}

fn open_in_file_manager(target: &Path) -> AppResult<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };

    // Explorer is a shell API consumer and rejects verbatim `\\?\C:\...`
    // paths; hand it the spelling the address bar accepts.
    let target = crate::spawn::native_path(target);
    std::process::Command::new(program)
        .arg(&target)
        .spawn()
        .map_err(|error| {
            AppError::msg(format!(
                "Could not open {} with {program}: {error}",
                target.display()
            ))
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    fn epub(path: &Path, chapters: usize) {
        let file = std::fs::File::create(path).expect("create epub");
        let mut zip = ZipWriter::new(file);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(br#"<container><rootfiles><rootfile full-path="EPUB/content.opf"/></rootfiles></container>"#).unwrap();
        let mut manifest = String::new();
        let mut spine = String::new();
        for index in 1..=chapters {
            manifest.push_str(&format!(
                r#"<item id="c{index}" href="chapter_{index:05}.xhtml" media-type="application/xhtml+xml"/>"#
            ));
            spine.push_str(&format!(r#"<itemref idref="c{index}"/>"#));
            zip.start_file(format!("EPUB/chapter_{index:05}.xhtml"), deflated)
                .unwrap();
            zip.write_all(
                format!(
                    "<html><body><h1>Chapter {index}</h1><p>{}</p></body></html>",
                    "word ".repeat(80)
                )
                .as_bytes(),
            )
            .unwrap();
        }
        zip.start_file("EPUB/content.opf", deflated).unwrap();
        zip.write_all(
            format!(
                r#"<package><metadata><title>Loose Book</title><creator>Author</creator></metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>"#
            )
            .as_bytes(),
        )
        .unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn classifies_loose_archives_and_skips_structured_downloads() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-archives-{}-{}",
            std::process::id(),
            crate::settings::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();

        let structured = root.join("structured");
        std::fs::create_dir_all(&structured).unwrap();
        std::fs::write(structured.join("metadata.json"), "{}").unwrap();
        epub(&structured.join("Native.epub"), 1);

        let loose = root.join("loose");
        std::fs::create_dir_all(&loose).unwrap();
        epub(&loose.join("Old Book.epub"), 2);

        let text = root.join("text-only");
        std::fs::create_dir_all(&text).unwrap();
        std::fs::write(text.join("notes.txt"), "not a book").unwrap();

        epub(&root.join("At the root.epub"), 1);

        let archives = library_archives(&root);
        assert_eq!(archives.len(), 3);

        let old = archives
            .iter()
            .find(|archive| archive.name == "Old Book.epub")
            .expect("loose epub");
        assert!(old.adoptable);
        assert!(old.reason.contains("Ready to adopt"));

        let notes = archives
            .iter()
            .find(|archive| archive.name == "notes.txt")
            .expect("text file");
        assert!(!notes.adoptable);
        assert!(notes.reason.contains("archive-only"));

        let root_file = archives
            .iter()
            .find(|archive| archive.name == "At the root.epub")
            .expect("root epub");
        assert!(!root_file.adoptable);
        assert!(root_file.reason.contains("own folder"));
        assert!(
            !archives
                .iter()
                .any(|archive| archive.folder == "structured")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn repack_builds_a_comic_archive_and_refuses_a_novel() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-repack-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let comic = root.join("comic-0123abcd");
        let page = comic.join("chapters/0001 - Chapter 1");
        std::fs::create_dir_all(&page).unwrap();
        std::fs::write(page.join("001.png"), [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a]).unwrap();
        std::fs::write(
            comic.join("metadata.json"),
            br#"{"title":"My Comic","epub":null}"#,
        )
        .unwrap();

        let archive = repack_structured_comic(&root, comic.to_str().unwrap(), "cbz")
            .expect("rebuild the comic archive");
        assert!(archive.ends_with("My Comic.cbz"), "{archive}");
        assert!(Path::new(&archive).is_file());

        // A novel folder already ships an EPUB; its `chapters/` holds XHTML, so
        // repacking it would produce an archive of prose, not pages.
        let novel = root.join("novel");
        std::fs::create_dir_all(novel.join("chapters")).unwrap();
        std::fs::write(novel.join("chapters/0001 - Chapter 1.xhtml"), b"<html/>").unwrap();
        std::fs::write(
            novel.join("metadata.json"),
            br#"{"title":"Book","epub":"Book.epub"}"#,
        )
        .unwrap();
        let error = repack_structured_comic(&root, novel.to_str().unwrap(), "cbz")
            .expect_err("a novel folder must be refused");
        assert!(error.to_string().contains("EPUB"), "{error}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
