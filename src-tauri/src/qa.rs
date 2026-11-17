//! Developer-only live download harness.
//!
//! Enabled in debug builds when `NOVELFORGE_QA_QUERY` is set. It runs the
//! real resolver, scraper and structured export pipeline without the webview
//! so the end-to-end artefact path can be exercised from a terminal.

use crate::model::{
    DownloadOptions, DownloadRequest, DuplicateMode, EVENT_LOG, Engine, LogEvent, LogStream,
    TaskStatus,
};
use crate::process::{self, AppState};
use std::time::Duration;
use tauri::{AppHandle, Listener};

pub fn requested_query() -> Option<(String, Option<String>, Engine)> {
    let query = std::env::var("NOVELFORGE_QA_QUERY").ok()?;
    let query = query.trim().to_string();
    if query.is_empty() {
        return None;
    }
    let chapters = std::env::var("NOVELFORGE_QA_CHAPTERS")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let engine = match std::env::var("NOVELFORGE_QA_ENGINE")
        .unwrap_or_else(|_| "webnovel".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "manga" => Engine::Manga,
        "books" | "book" => Engine::Books,
        _ => Engine::Webnovel,
    };
    Some((query, chapters, engine))
}

pub fn requested_index() -> Option<String> {
    std::env::var("NOVELFORGE_QA_INDEX_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn requested_transport() -> Option<String> {
    std::env::var("NOVELFORGE_QA_TRANSPORT_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Fetches one URL through the shared transport and prints its per-host
/// counters. This is the live probe for backoff/fallback behaviour.
pub async fn run_transport(url: String) {
    let started = std::time::Instant::now();
    let client = match crate::metadata::http_client() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("[qa] transport client failed: {error}");
            std::process::exit(2);
        }
    };
    match crate::transport::fetch(&client, &url, crate::transport::RequestKind::Toc).await {
        Ok(response) => {
            println!(
                "[qa] transport ok status={} via={} bytes={} request={:.2}s total={:.2}s",
                response.status,
                response.via,
                response.body.len(),
                response.elapsed.as_secs_f64(),
                started.elapsed().as_secs_f64()
            );
            println!(
                "[qa] transport report: {}",
                crate::transport::summary_for_url(&url)
            );
        }
        Err(error) => {
            eprintln!(
                "[qa] transport failed elapsed={:.2}s: {error}",
                started.elapsed().as_secs_f64()
            );
            println!(
                "[qa] transport report: {}",
                crate::transport::summary_for_url(&url)
            );
            std::process::exit(2);
        }
    }
}

pub async fn run_index(state: crate::process::AppState, url: String) {
    let started = std::time::Instant::now();
    let use_cache = std::env::var_os("NOVELFORGE_QA_INDEX_CACHE").is_some();
    println!(
        "[qa] fetching chapter index for {url} (mode={})",
        if use_cache { "cache" } else { "fresh" }
    );
    let settings = state
        .settings
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let result = crate::metadata::fetch_chapter_index_for(&settings, &url, !use_cache).await;
    match result {
        Ok(index) => {
            println!(
                "[qa] index ok total={} complete={} source={} cached={} age={:?} elapsed={:.1}s",
                index.total,
                index.complete,
                index.source,
                index.cached,
                index.cache_age_seconds,
                started.elapsed().as_secs_f64()
            );
            if let Some(detail) = index.detail.as_deref() {
                println!("[qa] index detail: {detail}");
            }
            if let Some(source) = index.title_source.as_deref() {
                println!("[qa] title source: {source}");
            }
            if let Some(transport) = index.transport.as_deref() {
                println!("[qa] transport: {transport}");
            }
            if let (Some(first), Some(last)) = (index.chapters.first(), index.chapters.last()) {
                println!(
                    "[qa] range {}..{} first={:?} last={:?}",
                    first.number, last.number, first.title, last.title
                );
            }
            for chapter in index.chapters.iter().take(3) {
                println!("[qa] chapter {} {:?}", chapter.number, chapter.title);
            }
            if let Some(chapter) = index.chapters.get(index.chapters.len() / 2) {
                println!("[qa] chapter {} {:?}", chapter.number, chapter.title);
            }
        }
        Err(error) => {
            eprintln!(
                "[qa] index failed elapsed={:.1}s: {error}",
                started.elapsed().as_secs_f64()
            );
            println!(
                "[qa] transport report: {}",
                crate::transport::summary_for_url(&url)
            );
            std::process::exit(2);
        }
    }
}

/// Developer-only override for transport tests. Accepts the same JSON shape as
/// `Settings.sourceLimits`, e.g. `[{"host":"127.0.0.1:8765","concurrency":1,
/// "requestGapMs":1234,"chapterRate":1.5}]`.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub fn apply_source_limits(mut settings: crate::settings::Settings) -> crate::settings::Settings {
    if let Ok(raw) = std::env::var("NOVELFORGE_QA_SOURCE_LIMITS")
        && let Ok(limits) = serde_json::from_str::<Vec<crate::settings::SourceLimit>>(raw.trim())
    {
        settings.source_limits = limits;
    }
    settings
}

pub fn requested_library() -> bool {
    std::env::var_os("NOVELFORGE_QA_LIBRARY").is_some()
}

pub fn requested_adopt() -> Option<String> {
    std::env::var("NOVELFORGE_QA_ADOPT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn requested_search() -> Option<(String, Vec<String>)> {
    let query = std::env::var("NOVELFORGE_QA_SEARCH").ok()?;
    let query = query.trim().to_string();
    if query.len() < 2 {
        return None;
    }
    let sources = std::env::var("NOVELFORGE_QA_SEARCH_SOURCES")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some((query, sources))
}

pub async fn run_search(app: AppHandle, query: String, sources: Vec<String>) {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let settings = apply_overrides(&state);
    let started = std::time::Instant::now();
    println!(
        "[qa] searching {query:?} sources={:?}",
        if sources.is_empty() {
            vec!["<novelforge default>".to_string()]
        } else {
            sources.clone()
        }
    );
    let response = crate::resolver::search_novels_from(&settings, &query, &sources).await;
    println!(
        "[qa] search done results={} searched={} failures={} cached={} cache_age={:?} elapsed={:.1}s",
        response.results.len(),
        response.searched.len(),
        response.failures.len(),
        response.cached,
        response.cache_age_seconds,
        started.elapsed().as_secs_f64()
    );
    for result in &response.results {
        println!(
            "[qa] result source={} title={:?} url={}",
            result.source, result.title, result.url
        );
    }
    for failure in &response.failures {
        println!(
            "[qa] failure source={} reason={}",
            failure.source, failure.reason
        );
    }
}

pub async fn run_library(app: AppHandle) {
    use tauri::Manager;
    let state = app.state::<AppState>();
    apply_overrides(&state);
    match crate::commands::list_library(state) {
        Ok(response) => match serde_json::to_string_pretty(&response) {
            Ok(json) => println!("{json}"),
            Err(error) => eprintln!("[qa] library serialise failed: {error}"),
        },
        Err(error) => {
            eprintln!("[qa] library failed: {error}");
            std::process::exit(2);
        }
    }
}

pub async fn run_adopt(app: AppHandle, path: String) {
    use tauri::Manager;
    let state = app.state::<AppState>();
    apply_overrides(&state);
    match crate::commands::adopt_archive(app.clone(), state, path.clone()) {
        Ok(novel) => match serde_json::to_string_pretty(&novel) {
            Ok(json) => println!("{json}"),
            Err(error) => eprintln!("[qa] adopted novel serialise failed: {error}"),
        },
        Err(error) => {
            eprintln!("[qa] adopt failed for {path}: {error}");
            std::process::exit(2);
        }
    }
}

pub fn requested_storage() -> Option<String> {
    std::env::var("NOVELFORGE_QA_STORAGE")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

/// Disk-usage report and the two explicit cleanup actions (NF-016). `work`
/// removes abandoned crawler trees, `partials` removes incomplete structured
/// folders, `all` runs both, `report` only measures.
pub async fn run_storage(app: AppHandle, action: String) {
    use tauri::Manager;
    let state = app.state::<AppState>();
    apply_overrides(&state);

    match action.as_str() {
        "report" => match crate::commands::list_library(state) {
            Ok(response) => {
                println!(
                    "[qa] storage library={} partial={} ({} folder(s)) work={} ({} folder(s)) quota={} over={}",
                    response.storage.library_bytes,
                    response.storage.partial_bytes,
                    response.storage.partial_folders,
                    response.storage.work_bytes,
                    response.storage.work_folders,
                    response.storage.quota_bytes,
                    response.storage.over_quota
                );
                for novel in &response.novels {
                    println!(
                        "[qa] novel folder={} title={:?} complete={} disk={}",
                        novel.folder, novel.title, novel.complete, novel.disk_bytes
                    );
                }
            }
            Err(error) => {
                eprintln!("[qa] storage report failed: {error}");
                std::process::exit(2);
            }
        },
        "work" => match crate::commands::cleanup_engine_work(app.clone(), state) {
            Ok(report) => print_cleanup("work", &report),
            Err(error) => {
                eprintln!("[qa] work cleanup failed: {error}");
                std::process::exit(2);
            }
        },
        "partials" => match crate::commands::delete_partial_downloads(app.clone(), state) {
            Ok(report) => print_cleanup("partials", &report),
            Err(error) => {
                eprintln!("[qa] partial cleanup failed: {error}");
                std::process::exit(2);
            }
        },
        "all" => {
            match crate::commands::cleanup_engine_work(app.clone(), state.clone()) {
                Ok(report) => print_cleanup("work", &report),
                Err(error) => eprintln!("[qa] work cleanup failed: {error}"),
            }
            match crate::commands::delete_partial_downloads(app.clone(), state) {
                Ok(report) => print_cleanup("partials", &report),
                Err(error) => eprintln!("[qa] partial cleanup failed: {error}"),
            }
        }
        other => {
            eprintln!("[qa] unknown storage action {other:?}; use report, work, partials or all");
            std::process::exit(2);
        }
    }
}

fn print_cleanup(action: &str, report: &crate::model::CleanupReport) {
    println!(
        "[qa] cleanup {action} removed={} kept={} bytes_freed={} errors={}",
        report.removed.len(),
        report.kept.len(),
        report.bytes_freed,
        report.errors.len()
    );
    for folder in &report.removed {
        println!("[qa] cleanup removed {folder}");
    }
    for folder in &report.kept {
        println!("[qa] cleanup kept {folder}");
    }
    for error in &report.errors {
        println!("[qa] cleanup error {error}");
    }
}

pub fn requested_book_search() -> Option<(String, String)> {
    let query = std::env::var("NOVELFORGE_QA_BOOKS").ok()?;
    let query = query.trim().to_string();
    if query.is_empty() {
        return None;
    }
    let mode = std::env::var("NOVELFORGE_QA_BOOKS_MODE").unwrap_or_else(|_| "auto".to_string());
    Some((query, mode))
}

pub async fn run_book_search(app: AppHandle, query: String, mode: String) {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let settings = apply_overrides(&state);
    let config = state.config_file.as_path().to_path_buf();
    println!("[qa] books query={query:?} mode={mode}");
    let started = std::time::Instant::now();
    match crate::books::search(&settings, &config, &query, &mode).await {
        Ok(response) => {
            println!(
                "[qa] books {}",
                serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string())
            );
        }
        Err(error) => eprintln!("[qa] books failed: {error}"),
    }
    println!("[qa] books elapsed={:.2}s", started.elapsed().as_secs_f64());
}

pub fn requested_resolve() -> Option<(String, String, Vec<String>)> {
    let query = std::env::var("NOVELFORGE_QA_RESOLVE").ok()?;
    let query = query.trim().to_string();
    if query.is_empty() {
        return None;
    }
    let mode = std::env::var("NOVELFORGE_QA_RESOLVE_MODE").unwrap_or_else(|_| "auto".to_string());
    let sources = std::env::var("NOVELFORGE_QA_RESOLVE_SOURCES")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some((query, mode, sources))
}

pub async fn run_resolve(app: AppHandle, query: String, mode: String, sources: Vec<String>) {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let settings = apply_overrides(&state);
    println!(
        "[qa] resolving {query:?} mode={mode} sources={:?}",
        if sources.is_empty() {
            vec!["<novelforge default>".to_string()]
        } else {
            sources.clone()
        }
    );
    let mode = crate::resolver::ResolveMode::parse(Some(&mode));
    let started = std::time::Instant::now();
    match crate::resolver::resolve_request_from(
        &settings,
        crate::model::Engine::Webnovel,
        &query,
        mode,
        &sources,
    )
    .await
    {
        Ok(match_) => {
            println!(
                "[qa] resolved title={:?} source={} confidence={:.2} matched_by={} detail={:?} cached={} cache_age={:?} elapsed={:.2}s",
                match_.title,
                match_.source,
                match_.confidence,
                match_.matched_by,
                match_.detail,
                match_.cached,
                match_.cache_age_seconds,
                started.elapsed().as_secs_f64()
            );
            println!("[qa] searched={:?}", match_.searched);
            for alternative in &match_.alternatives {
                println!(
                    "[qa] alternative title={:?} source={} url={}",
                    alternative.title, alternative.source, alternative.url
                );
            }
        }
        Err(error) => {
            eprintln!(
                "[qa] resolve failed after {:.2}s: {error}",
                started.elapsed().as_secs_f64()
            );
            std::process::exit(2);
        }
    }
}

fn env_trimmed(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn apply_overrides(state: &AppState) -> crate::settings::Settings {
    let mut settings = state
        .settings
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    if let Ok(path) = std::env::var("NOVELFORGE_QA_LNCRAWL")
        && !path.trim().is_empty()
    {
        settings.lncrawl_path = Some(path);
    }
    if let Ok(path) = std::env::var("NOVELFORGE_QA_GALLERY_DL")
        && !path.trim().is_empty()
    {
        settings.gallery_dl_path = Some(path);
    }
    if let Ok(directory) = std::env::var("NOVELFORGE_QA_DOWNLOAD_DIR")
        && !directory.trim().is_empty()
    {
        settings.download_dir = directory;
    }
    if let Ok(raw) = std::env::var("NOVELFORGE_QA_QUOTA_BYTES")
        && let Ok(bytes) = raw.trim().parse::<u64>()
    {
        settings.max_library_bytes = bytes;
    }
    if std::env::var_os("NOVELFORGE_QA_KEEP_WORK").is_some() {
        settings.cleanup_work_after_import = false;
    }
    if let Ok(mut guard) = state.settings.write() {
        *guard = settings.clone();
    }
    settings
}

pub fn requested_manga_sources() -> bool {
    std::env::var_os("NOVELFORGE_QA_MANGA_SOURCES").is_some()
}

/// Prints the live gallery-dl manga extractor catalogue as parsed by the app.
pub async fn run_manga_sources(app: AppHandle, state: AppState) {
    let settings = apply_overrides(&state);
    match crate::resolver::list_manga_sources(&settings).await {
        Ok(report) => {
            println!("[qa] manga sources={}", report.sources.len());
            for source in &report.sources {
                println!(
                    "[qa] source name={:?} language={:?} url={} tags={:?}",
                    source.name, source.language, source.url, source.tags
                );
            }
        }
        Err(error) => eprintln!("[qa] manga sources failed: {error}"),
    }
    let _ = app;
}

pub async fn run_download(
    app: AppHandle,
    state: AppState,
    query: String,
    chapters: Option<String>,
    engine: Engine,
) {
    let settings = apply_overrides(&state);

    // Mirror the app's log stream to the terminal so system timing lines and
    // crawler stderr are captured as live QA evidence.
    let _log_listener = app.listen(EVENT_LOG, |event| {
        if let Ok(record) = serde_json::from_str::<LogEvent>(event.payload()) {
            let prefix = match record.stream {
                LogStream::Stdout => "[qa:out]",
                LogStream::Stderr => "[qa:err]",
                LogStream::System => "[qa:sys]",
            };
            println!("{prefix} {}", record.line);
        }
    });

    println!(
        "[qa] starting {:?} query={:?} chapters={:?} download_dir={}",
        engine, query, chapters, settings.download_dir
    );

    let book_title = env_trimmed("NOVELFORGE_QA_BOOK_TITLE");
    let book_author = env_trimmed("NOVELFORGE_QA_BOOK_AUTHOR");
    let book_isbn = env_trimmed("NOVELFORGE_QA_BOOK_ISBN");
    let book_source = env_trimmed("NOVELFORGE_QA_BOOK_SOURCE");
    let book_series = env_trimmed("NOVELFORGE_QA_BOOK_SERIES");
    let book_series_position =
        env_trimmed("NOVELFORGE_QA_BOOK_POSITION").and_then(|value| value.parse::<f64>().ok());
    if engine == Engine::Books {
        println!(
            "[qa] book metadata title={book_title:?} author={book_author:?} isbn={book_isbn:?} source={book_source:?} series={book_series:?} position={book_series_position:?}"
        );
    }

    let request = DownloadRequest {
        engine,
        query,
        title: None,
        options: DownloadOptions {
            chapters,
            jobs: Some(2),
            throttle: Some(true),
            book_source,
            book_title,
            book_author,
            book_isbn,
            book_series,
            book_series_position,
            duplicate: std::env::var("NOVELFORGE_QA_DUPLICATE")
                .ok()
                .map(|value| value.trim().to_ascii_lowercase())
                .and_then(|value| match value.as_str() {
                    "update" => Some(DuplicateMode::Update),
                    "copy" => Some(DuplicateMode::Copy),
                    _ => None,
                }),
            target_folder: None,
        },
    };

    let task = match process::start_download(app.clone(), state.clone(), request).await {
        Ok(task) => task,
        Err(error) => {
            eprintln!("[qa] start failed: {error}");
            return;
        }
    };

    println!(
        "[qa] job {} resolved title={:?} destination={}",
        task.id, task.title, task.destination
    );
    wait_for_job(&app, &state, task).await;
}

pub fn requested_action() -> Option<(String, String, bool)> {
    let action = std::env::var("NOVELFORGE_QA_ACTION")
        .ok()?
        .trim()
        .to_ascii_lowercase();
    if action.is_empty() {
        return None;
    }
    let folder = std::env::var("NOVELFORGE_QA_FOLDER")
        .ok()?
        .trim()
        .to_string();
    if folder.is_empty() {
        return None;
    }
    let refresh = std::env::var("NOVELFORGE_QA_REFRESH")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });
    Some((action, folder, refresh))
}

pub async fn run_action(
    app: AppHandle,
    state: AppState,
    action: String,
    folder: String,
    refresh: bool,
) {
    let settings = apply_overrides(&state);
    let root = match crate::settings::prepare_download_dir(&settings) {
        Ok(root) => root,
        Err(error) => {
            eprintln!("[qa] action failed: {error}");
            std::process::exit(2);
        }
    };
    println!(
        "[qa] action={action} folder={folder} refresh={refresh} download_dir={}",
        settings.download_dir
    );

    match action.as_str() {
        "index" | "details" => match crate::library_actions::details(&root, &folder) {
            Ok(details) => {
                println!(
                    "[qa] details title={:?} chapters={} downloaded={} index_complete={} complete={} source={:?}",
                    details.title,
                    details.chapter_count,
                    details.downloaded_chapters,
                    details.index_complete,
                    details.complete,
                    details.source_url
                );
                if let Some(detail) = details.index_detail.as_deref() {
                    println!("[qa] index detail: {detail}");
                }
                if let (Some(first), Some(last)) =
                    (details.chapters.first(), details.chapters.last())
                {
                    println!(
                        "[qa] range {}..{} first={:?} last={:?}",
                        first.number, last.number, first.title, last.title
                    );
                }
                let downloaded = details
                    .chapters
                    .iter()
                    .filter(|chapter| chapter.downloaded)
                    .map(|chapter| chapter.number)
                    .collect::<Vec<_>>();
                println!("[qa] downloaded chapters: {downloaded:?}");
            }
            Err(error) => {
                eprintln!("[qa] details failed: {error}");
                std::process::exit(2);
            }
        },
        "chapters" => match crate::library_actions::chapter_folder(&root, &folder) {
            Ok(path) => println!("[qa] chapters folder={}", path.display()),
            Err(error) => {
                eprintln!("[qa] chapters failed: {error}");
                std::process::exit(2);
            }
        },
        "delete" => match crate::library_actions::delete_download(&app, &state, folder) {
            Ok(()) => println!("[qa] deleted"),
            Err(error) => {
                eprintln!("[qa] delete failed: {error}");
                std::process::exit(2);
            }
        },
        "retry" => {
            let task = match crate::library_actions::retry_download(
                app.clone(),
                state.clone(),
                folder,
                refresh,
            )
            .await
            {
                Ok(task) => task,
                Err(error) => {
                    eprintln!("[qa] retry failed: {error}");
                    std::process::exit(2);
                }
            };
            println!(
                "[qa] job {} retry title={:?} destination={}",
                task.id, task.title, task.destination
            );
            wait_for_job(&app, &state, task).await;
        }
        "update" => {
            let task = match crate::library_actions::update_download(
                app.clone(),
                state.clone(),
                folder,
                refresh,
            )
            .await
            {
                Ok(task) => task,
                Err(error) => {
                    eprintln!("[qa] update failed: {error}");
                    std::process::exit(2);
                }
            };
            println!(
                "[qa] job {} update title={:?} destination={}",
                task.id, task.title, task.destination
            );
            println!(
                "[qa] expected pace={:?} ch/min expected_seconds={:?}",
                task.expected_chapters_per_minute, task.expected_seconds
            );
            wait_for_job(&app, &state, task).await;
        }
        "faster" => {
            match crate::library_actions::suggest_faster_source(state.clone(), folder).await {
                Ok(report) => {
                    println!(
                        "[qa] faster current_host={:?} current_rank={} note={:?}",
                        report.current_host, report.current_rank, report.note
                    );
                    match report.alternative {
                        Some(candidate) => println!(
                            "[qa] alternative host={} rank={} title={:?} url={} detail={:?} confidence={:.2}",
                            candidate.host,
                            candidate.candidate_rank,
                            candidate.title,
                            candidate.url,
                            candidate.detail,
                            candidate.confidence
                        ),
                        None => println!("[qa] alternative none"),
                    }
                }
                Err(error) => {
                    eprintln!("[qa] faster failed: {error}");
                    std::process::exit(2);
                }
            }
        }
        other => {
            eprintln!("[qa] unknown action {other:?}");
            std::process::exit(2);
        }
    }
}

async fn wait_for_job(app: &AppHandle, state: &AppState, task: crate::model::DownloadTask) {
    let task_id = task.id.clone();
    let mut last_progress = -1.0_f32;
    let mut last_phase = task.phase;
    let mut last_bytes = u64::MAX;
    let pause_at = std::env::var("NOVELFORGE_QA_PAUSE_AT")
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok());
    let pause_for = std::env::var("NOVELFORGE_QA_PAUSE_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(5);
    let mut paused_once = false;

    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let tasks = process::list_tasks(state);
        let Some(current) = tasks.into_iter().find(|item| item.id == task_id) else {
            eprintln!("[qa] job disappeared from store");
            return;
        };

        if !paused_once
            && current.status == TaskStatus::Running
            && pause_at.is_some_and(|threshold| current.progress >= threshold)
        {
            paused_once = true;
            println!(
                "[qa] pausing at {:.0}% for {}s (elapsed={}s)",
                current.progress, pause_for, current.elapsed_seconds
            );
            match process::pause_download(app, state, &task_id) {
                Ok(paused) => println!(
                    "[qa] paused status={:?} elapsed={}s",
                    paused.status, paused.elapsed_seconds
                ),
                Err(error) => eprintln!("[qa] pause failed: {error}"),
            }
            tokio::time::sleep(Duration::from_secs(pause_for)).await;
            match process::resume_download(app, state, &task_id) {
                Ok(resumed) => println!(
                    "[qa] resumed status={:?} elapsed={}s eta={:?}",
                    resumed.status, resumed.elapsed_seconds, resumed.eta_seconds
                ),
                Err(error) => eprintln!("[qa] resume failed: {error}"),
            }
        }

        if (current.progress - last_progress).abs() >= 1.0
            || current.phase != last_phase
            || current.bytes != last_bytes
        {
            println!(
                "[qa] {:?} phase={:?} {:.0}% bytes={} speed={:.0}B/s eta={:?} elapsed={}s idle={}s",
                current.status,
                current.phase,
                current.progress,
                current.bytes,
                current.speed_bps,
                current.eta_seconds,
                current.elapsed_seconds,
                current.idle_seconds
            );
            last_progress = current.progress;
            last_phase = current.phase;
            last_bytes = current.bytes;
        }

        if current.status.is_terminal() {
            println!(
                "[qa] finished status={:?} phase={:?} elapsed={}s",
                current.status, current.phase, current.elapsed_seconds
            );
            if let Some(message) = &current.message {
                println!("[qa] message={message}");
            }
            for file in &current.files {
                println!(
                    "[qa] output {} ({} bytes, kind={}, verified={})",
                    file.path,
                    file.size,
                    file.kind,
                    file.verified.as_ref().is_some_and(|report| report.ok)
                );
            }
            println!("[qa] destination={}", current.destination);
            if url::Url::parse(&current.query).is_ok() {
                println!(
                    "[qa] transport report: {}",
                    crate::transport::summary_for_url(&current.query)
                );
            }
            // Give the job task a moment to deliver the final timing and
            // cleanup log lines before the harness exits the app.
            tokio::time::sleep(Duration::from_millis(900)).await;
            if current.status != TaskStatus::Completed {
                std::process::exit(2);
            }
            return;
        }
    }
}
