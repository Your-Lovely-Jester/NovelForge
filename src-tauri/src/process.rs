use crate::duplicates;
use crate::error::{AppError, AppResult};
use crate::export;
use crate::metadata;
use crate::model::*;
use crate::paths;
use crate::resolver;
use crate::settings::{Settings, now_ms, now_secs, prepare_download_dir, slugify};
use crate::spawn;
use crate::storage;
use crate::timeutil::civil_from_days;
use crate::tools::resolve_tool;
use crate::transport;
use crate::verify;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
use url::Url;
use uuid::Uuid;
use walkdir::WalkDir;

pub struct AppState {
    pub settings: Arc<RwLock<Settings>>,
    pub jobs: Arc<Mutex<HashMap<String, JobRecord>>>,
    pub config_file: Arc<PathBuf>,
    /// Durable job journal (`jobs.json`) so a close or crash never erases the
    /// Downloads list. Active jobs are restored as `Interrupted`.
    pub journal: Arc<PathBuf>,
    /// Append-only diagnostic log next to `settings.json`.
    pub log_file: Arc<PathBuf>,
    /// Set only from `confirm_quit`, so the window/exit guard can tell an
    /// approved exit from a stray close request.
    pub allow_exit: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(settings: Settings, config_file: PathBuf) -> Self {
        let directory = config_file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            settings: Arc::new(RwLock::new(settings)),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            config_file: Arc::new(config_file),
            journal: Arc::new(directory.join("jobs.json")),
            log_file: Arc::new(directory.join("novelforge.log")),
            allow_exit: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Reload the journal written by the previous session. Any job that was
    /// queued, running, paused or verifying becomes `Interrupted`; terminal
    /// rows are restored unchanged so the history survives a restart.
    pub fn restore_journal(&self) {
        let bytes = match std::fs::read(self.journal.as_ref()) {
            Ok(bytes) => bytes,
            Err(_) => return,
        };
        let journal: JobJournal = match serde_json::from_slice(&bytes) {
            Ok(journal) => journal,
            Err(error) => {
                eprintln!("jobs.json is invalid, starting with no tasks: {error}");
                return;
            }
        };
        let Ok(mut jobs) = self.jobs.lock() else {
            return;
        };
        let mut changed = false;
        for mut task in journal.jobs {
            if !task.status.is_terminal() {
                task.status = TaskStatus::Interrupted;
                task.speed_bps = 0.0;
                task.eta_seconds = None;
                task.message = Some(
                    "NovelForge closed while this download was running. Resume to continue."
                        .to_string(),
                );
                changed = true;
            }
            jobs.insert(
                task.id.clone(),
                JobRecord {
                    task,
                    pid: 0,
                    cancel: None,
                },
            );
        }
        drop(jobs);
        // Rewrite immediately so the on-disk journal never claims a task is
        // still running when nothing is backing it.
        if changed {
            self.persist_journal();
        }
    }

    /// Write every known task to `jobs.json` atomically. Called on status
    /// changes, inserts, clears and shutdown — never on a progress tick.
    pub fn persist_journal(&self) {
        let Ok(jobs) = self.jobs.lock() else {
            return;
        };
        let mut tasks = jobs
            .values()
            .map(|record| record.task.clone())
            .collect::<Vec<_>>();
        drop(jobs);
        tasks.sort_by_key(|task| task.created_at);
        let journal = JobJournal {
            schema_version: crate::model::SCHEMA_VERSION,
            jobs: tasks,
        };
        let Ok(json) = serde_json::to_string_pretty(&journal) else {
            return;
        };
        if let Err(error) = crate::fsutil::write_text_atomic(self.journal.as_ref(), &json) {
            eprintln!("Cannot write jobs.json: {error}");
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobJournal {
    schema_version: u32,
    #[serde(default)]
    jobs: Vec<DownloadTask>,
}

impl Clone for AppState {
    fn clone(&self) -> Self {
        Self {
            settings: Arc::clone(&self.settings),
            jobs: Arc::clone(&self.jobs),
            config_file: Arc::clone(&self.config_file),
            journal: Arc::clone(&self.journal),
            log_file: Arc::clone(&self.log_file),
            allow_exit: Arc::clone(&self.allow_exit),
        }
    }
}

pub struct JobRecord {
    pub task: DownloadTask,
    pub pid: u32,
    pub cancel: Option<oneshot::Sender<()>>,
}

/// Shared, lock-free signal from the crawler output pumps to the progress
/// sampler. Keeping it separate from `DownloadTask` means the sampler never has
/// to hold the job store lock while parsing or measuring.
struct JobProgress {
    percent: AtomicU32,
    chapters_done: AtomicU32,
    chapters_total: AtomicU32,
    phase: AtomicU8,
    last_activity_ms: AtomicU64,
}

impl JobProgress {
    fn new(phase: TaskPhase) -> Self {
        Self {
            percent: AtomicU32::new(0),
            chapters_done: AtomicU32::new(0),
            chapters_total: AtomicU32::new(0),
            phase: AtomicU8::new(phase.code()),
            last_activity_ms: AtomicU64::new(now_ms()),
        }
    }

    fn phase(&self) -> TaskPhase {
        TaskPhase::from_code(self.phase.load(Ordering::Relaxed))
    }

    fn set_phase(&self, phase: TaskPhase) {
        self.phase.store(phase.code(), Ordering::Relaxed);
    }

    fn reset_transfer(&self) {
        self.percent.store(0, Ordering::Relaxed);
        self.chapters_done.store(0, Ordering::Relaxed);
        self.chapters_total.store(0, Ordering::Relaxed);
    }

    fn touch(&self) {
        self.last_activity_ms.store(now_ms(), Ordering::Relaxed);
    }

    fn idle_seconds(&self) -> u64 {
        now_ms().saturating_sub(self.last_activity_ms.load(Ordering::Relaxed)) / 1_000
    }
}

/// Per-stage wall-clock timings, reported as one line when the job finishes.
#[derive(Default)]
struct JobTimings {
    crawl: Option<Duration>,
    metadata: Option<Duration>,
    import: Option<Duration>,
    index: Option<Duration>,
    audit: Option<Duration>,
    retry: Option<Duration>,
    export: Option<Duration>,
    verify: Option<Duration>,
    pack: Option<Duration>,
}

impl JobTimings {
    fn add(slot: &mut Option<Duration>, elapsed: Duration) {
        *slot = Some(slot.unwrap_or_default() + elapsed);
    }

    fn summary(&self, total: Duration) -> String {
        let part = |label: &str, value: Option<Duration>| match value {
            Some(value) => format!("{label} {}", format_duration(value)),
            None => format!("{label} —"),
        };
        [
            part("crawl", self.crawl),
            part("metadata", self.metadata),
            part("index", self.index),
            part("audit", self.audit),
            part("retry", self.retry),
            part("export", self.export),
            part("verify", self.verify),
            part("pack", self.pack),
            part("import", self.import),
            part("total", Some(total)),
        ]
        .join(" · ")
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs_f64();
    if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        let minutes = (seconds / 60.0).floor() as u64;
        let rest = seconds - minutes as f64 * 60.0;
        format!("{minutes}m {rest:04.1}s")
    }
}

fn input_is_url(input: &str) -> bool {
    Url::parse(input)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

pub async fn start_download(
    app: AppHandle,
    state: AppState,
    request: DownloadRequest,
) -> AppResult<DownloadTask> {
    let settings = state
        .settings
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default();

    validate_request(&request)?;
    let root = prepare_download_dir(&settings)?;
    let tool = resolve_tool(&settings, request.engine).ok_or_else(|| {
        AppError::msg(format!(
            "{} is not available. Open Guardrails to finish the toolchain setup.",
            request.engine.tool_id()
        ))
    })?;

    // NF-016: block a brand-new download before any network work when the
    // library is already at its quota. Retry/update target an existing folder
    // and stay available so the user can repair or complete it; "Keep both"
    // creates a new folder and is therefore checked.
    let is_recovery = request.options.target_folder.is_some()
        || matches!(request.options.duplicate, Some(DuplicateMode::Update));
    if !is_recovery
        && let Some(message) =
            storage::quota_block(storage::library_bytes(&root), settings.max_library_bytes)
    {
        return Err(AppError::msg(message));
    }

    let input = request.query.trim().to_string();
    let resolved = resolver::resolve_request(&settings, request.engine, &input).await?;
    // NF-018: a title that only weakly resembles a source hit must be confirmed
    // in the studio. The UI sends the chosen URL, which bypasses this guard;
    // an automated title job must never silently pick a guess.
    if request.engine == Engine::Webnovel
        && !input_is_url(&input)
        && resolved.confidence < resolver::CONFIRM_THRESHOLD
    {
        return Err(AppError::msg(format!(
            "Low-confidence match for “{input}”: “{}” on {} ({:.0}%). Resolve it in Webnovels and confirm the source, or paste the page URL.",
            resolved.title,
            resolved.source,
            resolved.confidence * 100.0
        )));
    }
    let title = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| resolved.title.clone());
    let id = Uuid::new_v4().to_string();
    let mut destination = root.join(format!("{}-{}", slugify(&title), &id[..8]));
    let mut duplicate_note: Option<String> = None;
    // "Keep both" reserves its folder early. If the preflight below fails, the
    // empty reservation is removed again instead of being left in the library.
    let mut reserved_destination = false;

    if request.engine == Engine::Books {
        // Prefer the catalogue title for the shelf path; the resolver title can
        // be the lowercased query (especially on the QA path).
        let shelf_title = request
            .options
            .book_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&title);
        if let Some(series_destination) =
            book_series_destination(&root, &request.options, shelf_title)
        {
            // A complete folder for this exact book is the re-run skip rule:
            // never create a second copy, and never silently overwrite it.
            if export::read_export_metadata(&series_destination)
                .map(|metadata| metadata.complete)
                .unwrap_or(false)
            {
                return Err(AppError::msg(format!(
                    "“{shelf_title}” is already complete in the library at {}. Delete or update it from Library to download it again.",
                    series_destination.display()
                )));
            }
            destination = series_destination;
        }
    }

    if request.engine == Engine::Webnovel {
        let target_folder = request
            .options
            .target_folder
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(target) = target_folder {
            // Library actions target one concrete card. Validate that folder
            // instead of falling back to the "newest match" heuristic.
            let canonical_root = paths::canonical_root(&root)?;
            let canonical = paths::contained_path(&canonical_root, Path::new(target))?;
            if canonical == canonical_root || paths::is_hidden_path(&canonical_root, &canonical) {
                return Err(AppError::msg(
                    "The selected download is not a structured NovelForge folder",
                ));
            }
            if !canonical.is_dir() {
                return Err(AppError::msg("The selected download is not a folder"));
            }
            let metadata = export::read_export_metadata(&canonical).ok_or_else(|| {
                AppError::msg(
                    "The selected folder has no readable metadata.json; it cannot be updated.",
                )
            })?;
            let target_url = metadata.source_url.as_deref().ok_or_else(|| {
                AppError::msg("The selected folder has no source URL; it cannot be updated.")
            })?;
            if export::normalize_source_url(target_url)
                != export::normalize_source_url(&resolved.source)
            {
                return Err(AppError::msg(
                    "The selected folder belongs to a different source URL.",
                ));
            }
            if folder_is_busy(&state, &canonical) {
                return Err(AppError::msg(
                    "That existing download is already being updated by a running job.",
                ));
            }
            destination = canonical;
            duplicate_note = Some(format!(
                "Updating selected structured folder · {}",
                destination.display()
            ));
        } else {
            let existing = duplicates::find_existing_download(&root, &resolved.source);
            match (existing, request.options.duplicate) {
                (Some(existing), None) => {
                    return Err(AppError::msg(format!(
                        "“{}” already exists at {}. Choose Update to refresh that folder or Keep both to create a separate copy.",
                        existing.title, existing.folder
                    )));
                }
                (Some(existing), Some(DuplicateMode::Update)) => {
                    let folder = PathBuf::from(&existing.folder);
                    if folder_is_busy(&state, &folder) {
                        return Err(AppError::msg(
                            "That existing download is already being updated by a running job.",
                        ));
                    }
                    destination = folder;
                    duplicate_note = Some(format!(
                        "Updating existing structured folder · {}{}",
                        destination.display(),
                        if existing.matches > 1 {
                            format!(
                                " · {} matching folders; using the newest complete one",
                                existing.matches
                            )
                        } else {
                            String::new()
                        }
                    ));
                }
                (Some(existing), Some(DuplicateMode::Copy)) => {
                    destination = duplicate_copy_destination(&root, &title)?;
                    reserved_destination = true;
                    duplicate_note = Some(format!(
                        "Keeping both · existing folder {} · new edition {}",
                        existing.folder,
                        destination.display()
                    ));
                }
                (None, _) => {}
            }
        }
    }
    // NF-044: a partial webnovel download sends `--chapters`, which lncrawl
    // filters by its 1-based catalogue serial. Fetch the list before the crawl
    // so the picker's numbers can be translated through the row order, and fail
    // before any folder or chapter is created when the list cannot prove the
    // mapping. The export's own index fetch reuses this cache entry.
    let chapter_index = if request.engine == Engine::Webnovel
        && normalize_chapters(request.options.chapters.as_deref())?.is_some()
    {
        let fetched = tokio::time::timeout(
            Duration::from_secs(300),
            metadata::fetch_chapter_numbers_for(&settings, &resolved.source),
        )
        .await;
        match fetched {
            Ok(Ok(index)) => Some(index),
            Ok(Err(error)) => {
                if reserved_destination {
                    let _ = std::fs::remove_dir(&destination);
                }
                return Err(error);
            }
            Err(_) => {
                if reserved_destination {
                    let _ = std::fs::remove_dir(&destination);
                }
                return Err(AppError::msg(
                    "The source chapter list timed out, so the selected chapters were not downloaded. Retry the selection.",
                ));
            }
        }
    } else {
        None
    };
    std::fs::create_dir_all(&destination)?;

    // Both external engines write beside the final folder. Webnovel gives
    // lncrawl a private data directory; manga gives gallery-dl a private
    // working tree that is packed into a CBZ before the audit.
    let work_dir = if matches!(request.engine, Engine::Webnovel | Engine::Manga) {
        let directory = root.join(".novelforge-work").join(&id);
        std::fs::create_dir_all(&directory)?;
        Some(directory)
    } else {
        None
    };

    let mut run_request = request.clone();
    run_request.query = resolved.source.clone();
    run_request.title = Some(title.clone());
    let args = match build_args(
        &run_request,
        &settings,
        &destination,
        chapter_index.as_ref(),
    ) {
        Ok(args) => args,
        Err(error) => {
            if reserved_destination {
                let _ = std::fs::remove_dir(&destination);
            }
            return Err(error);
        }
    };
    let command_line = tool.command_line(&args);
    // Keep everything needed to launch the engine again: NF-035 retries one
    // zero-output transient and resumes a stalled crawl.
    let spawner = CrawlSpawner {
        program: tool.program.clone(),
        prefix: tool.prefix.clone(),
        args: args.clone(),
        cwd: if request.engine == Engine::Manga {
            work_dir.as_deref().unwrap_or(&destination).to_path_buf()
        } else {
            destination.clone()
        },
        work_dir: work_dir.clone(),
        engine: request.engine,
    };
    let child = spawner.spawn().map_err(|error| {
        AppError::msg(format!(
            "Could not launch {} ({}): {error}",
            tool.label,
            tool.program.display()
        ))
    })?;
    let pid = child.id().unwrap_or(0);

    let task = DownloadTask {
        id: id.clone(),
        title: title.clone(),
        query: input.clone(),
        engine: request.engine,
        status: TaskStatus::Running,
        phase: TaskPhase::FetchingDetails,
        created_at: now_secs(),
        destination: destination.to_string_lossy().to_string(),
        progress: 0.0,
        bytes: 0,
        speed_bps: 0.0,
        eta_seconds: None,
        elapsed_seconds: 0,
        idle_seconds: 0,
        expected_chapters_per_minute: None,
        expected_seconds: None,
        message: None,
        files: Vec::new(),
    };

    let (cancel_tx, cancel_rx) = oneshot::channel();
    {
        let mut jobs = state
            .jobs
            .lock()
            .map_err(|_| AppError::msg("Job store unavailable"))?;
        prune_jobs(&mut jobs);
        jobs.insert(
            id.clone(),
            JobRecord {
                task: task.clone(),
                pid,
                cancel: Some(cancel_tx),
            },
        );
    }
    state.persist_journal();

    emit_task(&app, &task);
    log_system(
        &app,
        &id,
        format!("Launching {} · {}", tool.label, request.engine.label()),
    );
    log_system(&app, &id, format!("Resolved · {}", resolved.detail));
    if request.engine == Engine::Webnovel {
        log_system(
            &app,
            &id,
            format!(
                "Webnovel transport · {}",
                webnovel_transport_summary(&run_request.options, &run_request.query)
            ),
        );
    }
    log_system(&app, &id, format!("$ {command_line}"));
    log_system(&app, &id, format!("Sandbox: {}", destination.display()));
    let quota_note = if settings.max_library_bytes > 0 {
        format!(
            " of {} quota",
            storage::human_bytes(settings.max_library_bytes)
        )
    } else {
        " · no quota".to_string()
    };
    let selection_note = selected_chapter_count(request.options.chapters.as_deref())
        .map(|count| format!("{count} chapter(s)"))
        .unwrap_or_else(|| "all chapters".to_string());
    log_system(
        &app,
        &id,
        format!(
            "Library storage · {}{} · {} selected",
            storage::human_bytes(storage::library_bytes(&root)),
            quota_note,
            selection_note
        ),
    );
    if let Some(note) = &duplicate_note {
        log_system(&app, &id, note.clone());
    }
    if let Some(directory) = &work_dir {
        log_system(
            &app,
            &id,
            format!(
                "{} work dir: {}",
                if request.engine == Engine::Manga {
                    "gallery-dl"
                } else {
                    "lncrawl"
                },
                directory.display()
            ),
        );
        if request.engine == Engine::Webnovel {
            // The exact database lncrawl must create. If it ever fails again,
            // the log shows the URL instead of only sqlite's CANTOPEN error.
            let directory = spawn::native_path(directory);
            log_system(
                &app,
                &id,
                format!(
                    "lncrawl database · {}",
                    sqlite_url(&directory.join("sqlite.db"))
                ),
            );
        }
    }

    tokio::spawn(run_job(
        app,
        state.clone(),
        id,
        destination,
        work_dir,
        child,
        spawner,
        cancel_rx,
        request.engine,
        resolved.source.clone(),
        request.options.clone(),
        settings.clone(),
        title.clone(),
    ));

    Ok(task)
}

pub fn pause_download(app: &AppHandle, state: &AppState, job_id: &str) -> AppResult<DownloadTask> {
    let pid = {
        let jobs = state
            .jobs
            .lock()
            .map_err(|_| AppError::msg("Job store unavailable"))?;
        let record = jobs
            .get(job_id)
            .ok_or_else(|| AppError::msg("Unknown download job"))?;
        if record.task.status != TaskStatus::Running {
            return Err(AppError::msg("Only a running download can be paused"));
        }
        record.pid
    };

    pause_process(pid)?;
    let task = update_task(state, job_id, |task| {
        task.status = TaskStatus::Paused;
        task.speed_bps = 0.0;
        task.eta_seconds = None;
        task.message = Some("Paused by user".to_string());
    })
    .ok_or_else(|| AppError::msg("Unknown download job"))?;

    log_system(app, job_id, "Process paused");
    emit_task(app, &task);
    Ok(task)
}

pub fn resume_download(app: &AppHandle, state: &AppState, job_id: &str) -> AppResult<DownloadTask> {
    let pid = {
        let jobs = state
            .jobs
            .lock()
            .map_err(|_| AppError::msg("Job store unavailable"))?;
        let record = jobs
            .get(job_id)
            .ok_or_else(|| AppError::msg("Unknown download job"))?;
        if record.task.status != TaskStatus::Paused {
            return Err(AppError::msg("Only a paused download can be resumed"));
        }
        record.pid
    };

    resume_process(pid)?;
    let task = update_task(state, job_id, |task| {
        task.status = TaskStatus::Running;
        task.message = None;
    })
    .ok_or_else(|| AppError::msg("Unknown download job"))?;

    log_system(app, job_id, "Process resumed");
    emit_task(app, &task);
    Ok(task)
}

pub fn cancel_download(app: &AppHandle, state: &AppState, job_id: &str) -> AppResult<DownloadTask> {
    let (task, sender) = {
        let mut jobs = state
            .jobs
            .lock()
            .map_err(|_| AppError::msg("Job store unavailable"))?;
        let record = jobs
            .get_mut(job_id)
            .ok_or_else(|| AppError::msg("Unknown download job"))?;
        if record.task.status.is_terminal() {
            return Ok(record.task.clone());
        }
        (record.task.clone(), record.cancel.take())
    };

    if let Some(sender) = sender {
        let _ = sender.send(());
    }

    let task = update_task(state, job_id, |task| {
        task.message = Some("Stopping download…".to_string());
    })
    .unwrap_or(task);

    log_system(
        app,
        job_id,
        "Cancellation requested — terminating process group",
    );
    emit_task(app, &task);
    Ok(task)
}

pub fn clear_finished(state: &AppState) -> Vec<DownloadTask> {
    let mut tasks = {
        let Ok(mut jobs) = state.jobs.lock() else {
            return Vec::new();
        };
        jobs.retain(|_, record| !record.task.status.is_terminal());
        jobs.values()
            .map(|record| record.task.clone())
            .collect::<Vec<_>>()
    };
    tasks.sort_by_key(|task| task.created_at);
    state.persist_journal();
    tasks
}

/// Drop one journal row. Used when an interrupted task is resumed: the new
/// job replaces the stale row instead of leaving two entries for one folder.
pub fn discard_task(state: &AppState, job_id: &str) {
    if let Ok(mut jobs) = state.jobs.lock() {
        jobs.remove(job_id);
    }
    state.persist_journal();
}

pub fn list_tasks(state: &AppState) -> Vec<DownloadTask> {
    let Ok(jobs) = state.jobs.lock() else {
        return Vec::new();
    };
    let mut tasks = jobs
        .values()
        .map(|record| record.task.clone())
        .collect::<Vec<_>>();
    tasks.sort_by_key(|task| task.created_at);
    tasks
}

/// Job ids that are still queued, running or paused. Used by the work-tree
/// cleanup to avoid deleting state a live crawler is still writing.
pub fn active_job_ids(state: &AppState) -> std::collections::HashSet<String> {
    state
        .jobs
        .lock()
        .map(|jobs| {
            jobs.values()
                .filter(|record| !record.task.status.is_terminal())
                .map(|record| record.task.id.clone())
                .collect()
        })
        .unwrap_or_default()
}

pub fn shutdown_all(state: &AppState) {
    let pids = {
        let Ok(mut jobs) = state.jobs.lock() else {
            return;
        };
        let mut pids = Vec::new();
        for record in jobs.values_mut() {
            if record.pid > 0 && !record.task.status.is_terminal() {
                pids.push(record.pid);
                record.task.status = TaskStatus::Interrupted;
                record.task.speed_bps = 0.0;
                record.task.eta_seconds = None;
                record.task.message = Some(
                    "NovelForge closed while this download was running. Resume to continue."
                        .to_string(),
                );
            }
            record.cancel = None;
            record.pid = 0;
        }
        pids
    };
    // Record the interruption before the process dies: if the kill races the
    // shutdown, the journal still says the job was interrupted, not running.
    state.persist_journal();

    for pid in &pids {
        terminate_process(*pid);
    }
    if !pids.is_empty() {
        std::thread::sleep(Duration::from_millis(250));
        for pid in &pids {
            force_kill_process(*pid);
        }
    }
}

/// True when closing the window would abandon at least one live download.
pub fn quit_needs_confirmation(state: &AppState) -> bool {
    active_job_count(state) > 0
}

pub fn active_job_count(state: &AppState) -> usize {
    state
        .jobs
        .lock()
        .map(|jobs| {
            jobs.values()
                .filter(|record| !record.task.status.is_terminal())
                .count()
        })
        .unwrap_or(0)
}

fn pause_process(pid: u32) -> AppResult<()> {
    #[cfg(unix)]
    {
        signal_group(pid, libc::SIGSTOP);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err(AppError::msg(
            "Pause is not supported on this platform yet. Cancel and start the download again, or use a Unix build.",
        ))
    }
}

fn resume_process(pid: u32) -> AppResult<()> {
    #[cfg(unix)]
    {
        signal_group(pid, libc::SIGCONT);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err(AppError::msg(
            "Resume is not supported on this platform yet. Cancel and start the download again, or use a Unix build.",
        ))
    }
}

/// Ask a crawler process group to stop. Unix signals the whole group; Windows
/// uses `taskkill /T` so the Python child spawned by the launcher dies too.
fn terminate_process(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGCONT);
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("taskkill");
        command
            .args(["/PID", &pid.to_string(), "/T"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let _ = spawn::hide_console_std(&mut command).status();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

fn force_kill_process(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("taskkill");
        command
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let _ = spawn::hide_console_std(&mut command).status();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: i32) {
    if pid == 0 {
        return;
    }
    unsafe {
        libc::kill(-(pid as i32), signal);
    }
}

#[allow(clippy::too_many_arguments)]
/// NF-035: how long a crawl may go without a heartbeat before it is treated as
/// hung. lncrawl can legitimately print nothing and write nothing while it
/// walks a serial table of contents — 700 s was observed, and its request
/// budget allows longer — so the pre-chapter bound is deliberately generous.
/// Once chapter files are arriving, minutes of complete silence is a hang: at
/// the shipped 2 requests/s that is 300+ missed chapters.
const STALL_LIMIT_SECS: u64 = 900;
const CHAPTER_STALL_LIMIT_SECS: u64 = 300;
const STALL_POLL_SECS: u64 = 15;

/// Debug-only accelerator for the QA harness, which cannot wait 15 minutes to
/// prove the watchdog. Release builds compile this out and always use the
/// constants above. Accepts `NOVELFORGE_QA_STALL_SECS`.
fn qa_stall_override() -> Option<u64> {
    #[cfg(debug_assertions)]
    {
        std::env::var("NOVELFORGE_QA_STALL_SECS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|value| *value > 0)
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

fn stall_limit_seconds(phase: TaskPhase) -> u64 {
    if let Some(limit) = qa_stall_override() {
        return limit;
    }
    match phase {
        TaskPhase::DownloadingChapters | TaskPhase::DownloadingImages => CHAPTER_STALL_LIMIT_SECS,
        _ => STALL_LIMIT_SECS,
    }
}

fn stall_poll_duration() -> Duration {
    match qa_stall_override() {
        // Poll a few times inside the shortened window so the harness proves
        // the same code path quickly.
        Some(limit) => Duration::from_millis((limit * 250).max(100)),
        None => Duration::from_secs(STALL_POLL_SECS),
    }
}

/// `lncrawl --resume` selects only chapters it has not crawled. Everything
/// else about the invocation stays identical.
fn resume_args(args: &[String]) -> Vec<String> {
    let mut resumed = args.to_vec();
    if let Some(index) = resumed.iter().position(|arg| arg == "--all") {
        resumed[index] = "--resume".to_string();
    }
    resumed
}

/// Point lncrawl at the private job directory.
///
/// `LNCRAWL_DATA_PATH` sets `APP_DIR`, which holds the config, the novel store
/// and the SQLite database. v4.14 derives the database URL from `APP_DIR` as
/// `Path.resolve().as_posix()`, and a canonicalised Windows root arrives as
/// `\\?\C:\...`; the resulting `sqlite://///?/C:/...` URL made the very first
/// chapter fail with "unable to open database file". Both variables are set
/// here from the native (prefix-free) path so the database never depends on
/// that conversion. The retry path uses the same helper so a resumed crawl
/// lands in the same store.
fn lncrawl_data_env(command: &mut Command, directory: &Path) {
    let directory = spawn::native_path(directory);
    let database = sqlite_url(&directory.join("sqlite.db"));
    command
        .env("LNCRAWL_DATA_PATH", &directory)
        .env("DATABASE_URL", database);
}

/// `sqlite:///<absolute path>` in the forward-slash form SQLAlchemy expects.
fn sqlite_url(path: &Path) -> String {
    format!("sqlite:///{}", path.to_string_lossy().replace('\\', "/"))
}

/// Everything needed to launch the engine again. NF-035 retries a transient
/// failure once; without this the first `Child` would be the only chance.
struct CrawlSpawner {
    program: PathBuf,
    prefix: Vec<String>,
    args: Vec<String>,
    cwd: PathBuf,
    work_dir: Option<PathBuf>,
    engine: Engine,
}

impl CrawlSpawner {
    fn spawn(&self) -> std::io::Result<Child> {
        self.spawn_with(&self.args)
    }

    /// Resume rather than start over. Used after a stall, where chapters may
    /// already be in the work directory; lncrawl's `--resume` selects only the
    /// chapters it has not crawled.
    fn spawn_resume(&self) -> std::io::Result<Child> {
        self.spawn_with(&resume_args(&self.args))
    }

    fn spawn_with(&self, args: &[String]) -> std::io::Result<Child> {
        let mut command = Command::new(spawn::native_path(&self.program));
        command
            .args(&self.prefix)
            .args(args)
            .current_dir(spawn::native_path(&self.cwd))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env("NO_COLOR", "1")
            .env("FORCE_COLOR", "0")
            .env("TERM", "dumb")
            // Rich wraps long artifact paths at its detected width; widen it so
            // the printed output path stays on one line and can be captured.
            .env("COLUMNS", "240")
            .env("PYTHONUNBUFFERED", "1");
        if self.engine == Engine::Webnovel
            && let Some(directory) = &self.work_dir
        {
            lncrawl_data_env(&mut command, directory);
        }
        // aget's OceanOfPDF driver needs an explicit Chromium. The bundled
        // Playwright browser is the only one the toolchain guarantees.
        if self.engine == Engine::Books
            && std::env::var_os("NOVELFORGE_CHROMIUM").is_none()
            && let Some(chromium) = crate::tools::playwright_chromium_path()
        {
            command.env("NOVELFORGE_CHROMIUM", chromium);
        }

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        spawn::hide_console(&mut command);

        command.spawn()
    }
}

struct PumpHandles {
    stdout: Option<tokio::task::JoinHandle<()>>,
    stderr: Option<tokio::task::JoinHandle<()>>,
}

fn spawn_pumps(
    child: &mut Child,
    app: &AppHandle,
    job_id: &str,
    progress: &Arc<JobProgress>,
    artifacts: &Arc<Mutex<ArtifactCapture>>,
    error_tail: &Arc<Mutex<ErrorTail>>,
) -> PumpHandles {
    let stdout = child.stdout.take().map(|stdout| {
        tokio::spawn(pump_lines(
            stdout,
            LogStream::Stdout,
            app.clone(),
            job_id.to_string(),
            Arc::clone(progress),
            Some(Arc::clone(artifacts)),
            Some(Arc::clone(error_tail)),
        ))
    });
    let stderr = child.stderr.take().map(|stderr| {
        tokio::spawn(pump_lines(
            stderr,
            LogStream::Stderr,
            app.clone(),
            job_id.to_string(),
            Arc::clone(progress),
            None,
            Some(Arc::clone(error_tail)),
        ))
    });
    PumpHandles { stdout, stderr }
}

async fn drain_pumps(handles: PumpHandles) {
    if let Some(handle) = handles.stdout {
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
    }
    if let Some(handle) = handles.stderr {
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
    }
}

enum CrawlExit {
    Cancelled,
    Exited(std::process::ExitStatus),
    WaitError(std::io::Error),
    Stalled { idle: u64, limit: u64 },
}

/// Wait for the crawl to finish, be cancelled, or go silent. The heartbeat is
/// `JobProgress`, which every log line, byte of growth and resume already
/// touches, so a static UI and a hung process are the same condition.
async fn supervise<F>(
    child: &mut Child,
    pid: u32,
    cancel_rx: &mut oneshot::Receiver<()>,
    progress: &Arc<JobProgress>,
    is_paused: F,
    poll: Duration,
) -> CrawlExit
where
    F: Fn() -> bool,
{
    tokio::select! {
        _ = &mut *cancel_rx => {
            terminate(child, pid).await;
            let _ = child.wait().await;
            CrawlExit::Cancelled
        }
        result = child.wait() => {
            match result {
                Ok(status) => CrawlExit::Exited(status),
                Err(error) => CrawlExit::WaitError(error),
            }
        }
        (idle, limit) = stall_watch_with(Arc::clone(progress), &is_paused, poll) => {
            terminate(child, pid).await;
            let _ = child.wait().await;
            CrawlExit::Stalled { idle, limit }
        }
    }
}

fn job_is_paused(state: &AppState, job_id: &str) -> bool {
    state
        .jobs
        .lock()
        .ok()
        .and_then(|jobs| {
            jobs.get(job_id)
                .map(|record| record.task.status == TaskStatus::Paused)
        })
        .unwrap_or(false)
}

async fn stall_watch_with<F>(progress: Arc<JobProgress>, is_paused: F, poll: Duration) -> (u64, u64)
where
    F: Fn() -> bool,
{
    let mut tick = tokio::time::interval(poll);
    tick.tick().await; // the first tick completes immediately
    // Time spent paused is not silence from the source. Without this, resuming
    // a job that sat paused for an hour would trip the watchdog on its first
    // tick, before the engine could print anything.
    let mut paused_total = Duration::ZERO;
    let mut paused_at: Option<Instant> = None;
    loop {
        tick.tick().await;
        if is_paused() {
            paused_at.get_or_insert_with(Instant::now);
            continue;
        }
        if let Some(since) = paused_at.take() {
            paused_total += since.elapsed();
        }
        let idle = progress
            .idle_seconds()
            .saturating_sub(paused_total.as_secs());
        let limit = stall_limit_seconds(progress.phase());
        if idle >= limit {
            return (idle, limit);
        }
    }
}

/// NF-042: the final message when a finished crawl produced no output. A
/// precise reason captured from stdout stands on its own; anything else gets
/// the generic sentence appended.
fn zero_output_message(message: Option<String>) -> String {
    match message {
        Some(note) if note.starts_with("Source error · ") => note,
        Some(note) => format!("{note} No usable chapter was produced."),
        None => "Process finished without producing a usable output file.".to_string(),
    }
}

/// Any chapter body or export at all means the crawl was doing real work.
/// Retrying from scratch would discard that; a stall is resumed instead.
fn crawl_has_output(destination: &Path, work_dir: Option<&Path>, engine: Engine) -> bool {
    if scan_outputs(destination).iter().any(|path| path.is_file()) {
        return true;
    }
    let structured = work_stats(destination, engine);
    if structured.chapters > 0 || structured.images > 0 {
        return true;
    }
    work_dir.is_some_and(|directory| {
        let stats = work_stats(directory, engine);
        stats.chapters > 0 || stats.images > 0
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryKind {
    /// Chapters are already on disk: continue what lncrawl has not crawled.
    Resume,
    /// Nothing was written: the same invocation gets one more chance.
    Restart,
}

/// NF-035's one automatic retry, and what it is allowed to do:
///
/// - a stall is resumed, because the partial work is valuable;
/// - a zero-output transient failure starts over, because there is nothing to
///   resume;
/// - validation failures, cancellations and a second attempt never retry.
fn retry_action(
    engine: Engine,
    attempt: u32,
    status: TaskStatus,
    has_output: bool,
    stalled: bool,
    transient: bool,
) -> Option<RetryKind> {
    if engine != Engine::Webnovel || attempt != 1 || status != TaskStatus::Failed {
        return None;
    }
    if stalled {
        return Some(RetryKind::Resume);
    }
    if !has_output && transient {
        return Some(RetryKind::Restart);
    }
    None
}

#[allow(clippy::too_many_arguments)]
async fn run_job(
    app: AppHandle,
    state: AppState,
    job_id: String,
    mut destination: PathBuf,
    work_dir: Option<PathBuf>,
    mut child: Child,
    spawner: CrawlSpawner,
    mut cancel_rx: oneshot::Receiver<()>,
    engine: Engine,
    source_url: String,
    options: DownloadOptions,
    settings: Settings,
    mut title: String,
) {
    let started = Instant::now();
    let started_at = SystemTime::now();
    let progress = Arc::new(JobProgress::new(TaskPhase::FetchingDetails));
    if let Some(total) = selected_chapter_count(options.chapters.as_deref()) {
        progress.chapters_total.store(total, Ordering::Relaxed);
    }
    let done = Arc::new(AtomicBool::new(false));
    let artifacts: Arc<Mutex<ArtifactCapture>> = Arc::new(Mutex::new(ArtifactCapture::default()));
    let error_tail = Arc::new(Mutex::new(ErrorTail::default()));
    let mut pid = child.id().unwrap_or(0);
    let mut timings = JobTimings::default();

    let poller = tokio::spawn(progress_loop(
        app.clone(),
        state.clone(),
        job_id.clone(),
        destination.clone(),
        work_dir.clone(),
        engine,
        Arc::clone(&progress),
        Arc::clone(&done),
        started,
    ));

    // NF-039: lncrawl stores each chapter body as soon as it is extracted.
    // Structure those files while the crawl continues, so a long download is
    // readable and crash-safe instead of only appearing at the final export.
    let stream_stop = Arc::new(AtomicBool::new(false));
    let structurer = if engine == Engine::Webnovel {
        work_dir.as_ref().map(|directory| {
            let logger_app = app.clone();
            let logger_job = job_id.clone();
            crate::structurer::spawn(
                destination.clone(),
                directory.clone(),
                source_url.clone(),
                Arc::clone(&stream_stop),
                Some(Box::new(move |message: String| {
                    log_system(&logger_app, &logger_job, message);
                })),
            )
        })
    } else {
        None
    };

    let mut outcome: JobOutcome;
    let mut attempt: u32 = 1;
    let mut retried = false;

    loop {
        let handles = spawn_pumps(
            &mut child,
            &app,
            &job_id,
            &progress,
            &artifacts,
            &error_tail,
        );
        let exit = supervise(
            &mut child,
            pid,
            &mut cancel_rx,
            &progress,
            || job_is_paused(&state, &job_id),
            stall_poll_duration(),
        )
        .await;
        drain_pumps(handles).await;
        if attempt == 1 {
            timings.crawl = Some(started.elapsed());
        } else {
            JobTimings::add(
                &mut timings.retry,
                started.elapsed() - timings.crawl.unwrap_or_default(),
            );
        }

        let mut stalled = false;
        outcome = match exit {
            CrawlExit::Cancelled => JobOutcome {
                status: TaskStatus::Cancelled,
                message: Some("Download cancelled by user".to_string()),
            },
            CrawlExit::Exited(status) if status.success() => {
                // NF-042: lncrawl can print a fatal error on stdout and still
                // exit 0. If no artifact exists, carry that reason into the
                // failure message instead of only reporting "no usable file".
                let reason = error_tail.lock().ok().and_then(|tail| tail.summary());
                let message =
                    reason.filter(|_| !crawl_has_output(&destination, work_dir.as_deref(), engine));
                JobOutcome {
                    status: TaskStatus::Verifying,
                    message: message.map(|reason| format!("Source error · {reason}")),
                }
            }
            CrawlExit::Exited(status) => JobOutcome {
                status: TaskStatus::Failed,
                // Filled in after the stderr pump drains, so the message can
                // name the engine's own reason instead of only the exit code.
                message: Some(format!(
                    "Process exited with code {}",
                    status
                        .code()
                        .map_or_else(|| "signal".to_string(), |code| code.to_string())
                )),
            },
            CrawlExit::WaitError(error) => JobOutcome {
                status: TaskStatus::Failed,
                message: Some(format!("Process supervision error: {error}")),
            },
            CrawlExit::Stalled { idle, limit } => {
                // NF-035: a crawl that prints nothing, writes nothing and does
                // not change phase for the whole window is hung, not slow. Kill
                // it so the user is not left with a static bar for hours.
                stalled = true;
                JobOutcome {
                    status: TaskStatus::Failed,
                    message: Some(format!(
                        "Source stalled: no output, bytes or progress for {} (limit {}). Work directory kept.",
                        format_duration(Duration::from_secs(idle)),
                        format_duration(Duration::from_secs(limit)),
                    )),
                }
            }
        };

        if attempt >= 2 || outcome.status != TaskStatus::Failed {
            break;
        }
        let has_output = crawl_has_output(&destination, work_dir.as_deref(), engine);
        let transient = error_tail
            .lock()
            .ok()
            .is_some_and(|tail| tail.is_transient());
        let Some(kind) = retry_action(
            engine,
            attempt,
            outcome.status,
            has_output,
            stalled,
            transient,
        ) else {
            break;
        };

        // A stall resumes from the chapters already on disk; a clean zero-output
        // transient starts over. Validation failures never reach this point.
        let next = match kind {
            RetryKind::Resume => {
                log_system(
                    &app,
                    &job_id,
                    "Stalled with partial output — resuming missing chapters (attempt 2/2)"
                        .to_string(),
                );
                spawner.spawn_resume()
            }
            RetryKind::Restart => {
                log_system(
                    &app,
                    &job_id,
                    "Crawl failed with no output but a transient error — retrying once from scratch (attempt 2/2)".to_string(),
                );
                spawner.spawn()
            }
        };
        match next {
            Ok(next) => {
                child = next;
                pid = child.id().unwrap_or(0);
                if let Ok(mut jobs) = state.jobs.lock()
                    && let Some(record) = jobs.get_mut(&job_id)
                {
                    record.pid = pid;
                }
                retried = true;
                attempt += 1;
                if let Ok(mut tail) = error_tail.lock() {
                    *tail = ErrorTail::default();
                }
                if let Ok(mut capture) = artifacts.lock() {
                    *capture = ArtifactCapture::default();
                }
                progress.reset_transfer();
                if let Some(total) = selected_chapter_count(options.chapters.as_deref()) {
                    progress.chapters_total.store(total, Ordering::Relaxed);
                }
                progress.set_phase(TaskPhase::FetchingDetails);
                progress.touch();
            }
            Err(error) => {
                outcome.message = Some(format!(
                    "{} (the automatic retry could not start: {error})",
                    outcome.message.unwrap_or_default()
                ));
                break;
            }
        }
    }

    if retried
        && outcome.status == TaskStatus::Failed
        && let Some(message) = outcome.message.as_mut()
    {
        message.push_str(" · one automatic retry also failed");
    }

    let licensing_note = error_tail
        .lock()
        .ok()
        .and_then(|tail| tail.licensing_note());
    let licensing_detail = error_tail
        .lock()
        .ok()
        .and_then(|tail| tail.licensing_detail());
    if let Some(detail) = &licensing_detail {
        // Full publisher links stay in the dock; the job row gets the short form.
        log_system(&app, &job_id, detail);
    }

    if outcome.status == TaskStatus::Failed {
        if let Some(note) = licensing_note.clone() {
            outcome.message = Some(note);
        } else if let Some(message) = outcome.message.as_mut()
            && message.starts_with("Process exited with code ")
        {
            let detail = error_tail.lock().ok().and_then(|tail| tail.summary());
            if let Some(detail) = detail {
                message.push_str(" · ");
                message.push_str(&detail);
            }
        }
    } else if outcome.status == TaskStatus::Verifying {
        // NF-042: the source reason gathered from stdout must survive
        // verification. Only a licensing note replaces it; previously this
        // branch assigned `None` and silently dropped the real cause.
        if let Some(note) = licensing_note.clone() {
            outcome.message = Some(note);
        }
    }

    if engine == Engine::Manga && outcome.status == TaskStatus::Verifying {
        set_phase(&state, &app, &job_id, TaskPhase::Packing);
        let pack_started = Instant::now();
        let source_root = work_dir
            .as_deref()
            .map(|directory| directory.join("gallery-dl"));

        // MangaDex-style URLs end in an opaque UUID, which is a poor folder
        // name. gallery-dl knows the series title, so adopt it once the tree
        // exists and rename the still-empty sandbox before packing.
        if let Some(root) = source_root.as_deref()
            && is_opaque_title(&title)
            && let Some(series) = gallery_dl_series_title(root)
        {
            let renamed =
                destination.with_file_name(format!("{}-{}", slugify(&series), &job_id[..8]));
            match std::fs::rename(&destination, &renamed) {
                Ok(()) => {
                    log_system(&app, &job_id, format!("Series title · {series}"));
                    destination = renamed;
                    title = series;
                    let updated = state.jobs.lock().ok().and_then(|mut jobs| {
                        let record = jobs.get_mut(&job_id)?;
                        record.task.destination = destination.to_string_lossy().to_string();
                        Some(record.task.clone())
                    });
                    if let Some(task) = updated {
                        emit_task(&app, &task);
                    }
                }
                Err(error) => log_system(
                    &app,
                    &job_id,
                    format!("Could not rename manga sandbox: {error}"),
                ),
            }
        }

        let packed = match source_root {
            Some(root) => {
                export::package_manga_directory(&root, &destination, &settings.manga_format, &title)
            }
            None => Err("gallery-dl work directory is missing".to_string()),
        };
        match packed {
            Ok(path) => log_system(
                &app,
                &job_id,
                format!(
                    "Packed {} · {}",
                    file_name(&path),
                    format_duration(pack_started.elapsed())
                ),
            ),
            Err(error) => {
                log_stderr(&app, &job_id, format!("Manga packaging failed: {error}"));
                outcome = JobOutcome {
                    status: TaskStatus::Failed,
                    message: Some(error),
                };
            }
        }
        JobTimings::add(&mut timings.pack, pack_started.elapsed());
    }

    if engine == Engine::Webnovel && outcome.status == TaskStatus::Verifying {
        set_phase(&state, &app, &job_id, TaskPhase::Importing);
        let import_started = Instant::now();
        import_lncrawl_artifacts(
            &app,
            &job_id,
            &artifacts,
            &destination,
            started_at,
            work_dir.as_deref(),
        );
        JobTimings::add(&mut timings.import, import_started.elapsed());
    }

    // Stop the live structurer before the authoritative export: the export
    // sweeps provisional files, so nothing may recreate one afterwards. The
    // poller exits within one poll interval.
    stream_stop.store(true, Ordering::Relaxed);
    if let Some(handle) = structurer {
        let _ = handle.await;
    }

    let cleanup_work = settings.cleanup_work_after_import;
    let status = finalize(
        app.clone(),
        state,
        job_id.clone(),
        destination,
        engine,
        outcome,
        source_url,
        options,
        settings,
        title,
        &mut timings,
        work_dir.as_deref(),
        Arc::clone(&progress),
    )
    .await;

    let total = started.elapsed();
    done.store(true, Ordering::Relaxed);
    let _ = tokio::time::timeout(Duration::from_millis(1_200), poller).await;
    log_system(
        &app,
        &job_id,
        format!("Timing · {}", timings.summary(total)),
    );

    if status == TaskStatus::Completed && cleanup_work {
        if let Some(directory) = &work_dir {
            match std::fs::remove_dir_all(directory) {
                Ok(()) => {
                    log_system(
                        &app,
                        &job_id,
                        format!("Removed work dir {}", directory.display()),
                    );
                    // Drop the shared container once the last job is gone so a
                    // clean success leaves no trace under the download root.
                    if let Some(parent) = directory.parent() {
                        let _ = std::fs::remove_dir(parent);
                    }
                }
                Err(error) => log_system(
                    &app,
                    &job_id,
                    format!("Could not remove work dir {}: {error}", directory.display()),
                ),
            }
        }
    } else if status == TaskStatus::Completed
        && let Some(directory) = &work_dir
    {
        log_system(
            &app,
            &job_id,
            format!(
                "Kept work dir {} (work-file cleanup disabled in Settings)",
                directory.display()
            ),
        );
    }
}

struct JobOutcome {
    status: TaskStatus,
    message: Option<String>,
}

fn destination_has_epub(destination: &Path) -> bool {
    scan_outputs(destination).iter().any(|path| {
        path.extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("epub"))
    })
}

#[allow(clippy::too_many_arguments)]
async fn finalize(
    app: AppHandle,
    state: AppState,
    job_id: String,
    destination: PathBuf,
    engine: Engine,
    mut outcome: JobOutcome,
    source_url: String,
    options: DownloadOptions,
    settings: Settings,
    title: String,
    timings: &mut JobTimings,
    work_dir: Option<&Path>,
    progress: Arc<JobProgress>,
) -> TaskStatus {
    if outcome.status == TaskStatus::Cancelled {
        let files = scan_outputs(&destination)
            .into_iter()
            .map(|path| output_file(&path, None))
            .collect::<Vec<_>>();
        let task = update_task(&state, &job_id, |task| {
            task.status = TaskStatus::Cancelled;
            task.message = outcome
                .message
                .clone()
                .or_else(|| Some("Download cancelled".to_string()));
            task.speed_bps = 0.0;
            task.eta_seconds = None;
            task.files = files;
        });
        if let Some(task) = task {
            log_system(
                &app,
                &job_id,
                "Job cancelled. Partial files were left in place.",
            );
            emit_task(&app, &task);
        }
        return TaskStatus::Cancelled;
    }

    let mut status = if outcome.status == TaskStatus::Failed {
        TaskStatus::Failed
    } else {
        TaskStatus::Verifying
    };

    // A books failure with no EPUB is a "not found", not a transport mystery.
    // Saying so keeps a series run explainable while the other books continue.
    if engine == Engine::Books
        && outcome.status == TaskStatus::Failed
        && !destination_has_epub(&destination)
    {
        outcome.message = Some(
            "Book not found: no catalogue-verified EPUB was produced (network failure, mirror block, or no row cleared the match threshold). Check the near-miss log, the ISBN, or try another source.".to_string(),
        );
    }

    set_phase(&state, &app, &job_id, TaskPhase::Auditing);
    let verifying = update_task(&state, &job_id, |task| {
        task.status = TaskStatus::Verifying;
        task.progress = 100.0;
        task.message = Some("Verifying chapters and building structured output…".to_string());
    });
    if let Some(task) = &verifying {
        emit_task(&app, task);
    }
    log_system(
        &app,
        &job_id,
        "Auditing chapter files and building the structured export",
    );

    let audit_started = Instant::now();
    let scanned = scan_outputs(&destination);
    log_system(
        &app,
        &job_id,
        format!("Output scan found {} file(s)", scanned.len()),
    );
    let primary = choose_primary(&scanned, engine);
    JobTimings::add(&mut timings.audit, audit_started.elapsed());
    let mut outputs = Vec::new();
    let mut rejection = None;
    let mut incomplete: Option<String> = None;
    let mut export_failed: Option<String> = None;

    if let Some(primary_path) = primary.clone() {
        match engine {
            Engine::Webnovel => {
                process_epub_download(
                    &app,
                    &state,
                    &job_id,
                    &destination,
                    &settings,
                    &source_url,
                    &options,
                    engine,
                    &primary_path,
                    &mut outputs,
                    &mut incomplete,
                    &mut export_failed,
                    timings,
                    work_dir,
                    Arc::clone(&progress),
                )
                .await;
            }
            Engine::Books => {
                process_book_download(
                    &app,
                    &state,
                    &job_id,
                    &destination,
                    engine,
                    &options,
                    &primary_path,
                    &mut outputs,
                    &mut incomplete,
                    &mut export_failed,
                    timings,
                )
                .await;
            }
            Engine::Manga => {
                process_manga_download(
                    &app,
                    &state,
                    &job_id,
                    &destination,
                    &source_url,
                    &options,
                    &primary_path,
                    &title,
                    work_dir,
                    &mut outputs,
                    &mut incomplete,
                    &mut export_failed,
                    timings,
                )
                .await;
            }
        }
    }

    // Verify any remaining artifacts that the structured pipeline did not
    // consume. An older EPUB from a retry is removed so the final folder only
    // contains one bundled book.
    for path in scanned {
        if primary.as_ref().is_some_and(|value| value == &path) {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if engine == Engine::Webnovel && extension == "epub" {
            let _ = std::fs::remove_file(&path);
            log_system(
                &app,
                &job_id,
                format!("Removed superseded retry artifact {}", file_name(&path)),
            );
            continue;
        }
        let report = verify::verify_any(&path, engine == Engine::Books);
        if !report.ok {
            let _ = std::fs::remove_file(&path);
            log_stderr(
                &app,
                &job_id,
                format!(
                    "SECURITY: rejected and deleted {} — {}",
                    file_name(&path),
                    report.detail
                ),
            );
            rejection = Some(format!("Rejected {}: {}", file_name(&path), report.detail));
            continue;
        }
        log_system(
            &app,
            &job_id,
            format!("Verified {} — {}", file_name(&path), report.detail),
        );
        outputs.push(output_file(&path, Some(report)));
    }

    // A retry can leave the previous partial EPUB beside the new one. Keep only
    // the artifact that made it into the final output list.
    if engine == Engine::Webnovel {
        let kept = outputs
            .iter()
            .map(|output| PathBuf::from(&output.path))
            .collect::<Vec<_>>();
        for path in scan_outputs(&destination) {
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("epub"))
                && !kept.contains(&path)
            {
                let _ = std::fs::remove_file(&path);
                log_system(
                    &app,
                    &job_id,
                    format!("Removed superseded artifact {}", file_name(&path)),
                );
            }
        }
    }

    let mut message = outcome.message;
    // The licensing note explains the cause; the audit explains the damage.
    // Keep both when an incomplete range is also a licensed one. `run_job`
    // already put the note in the outcome message, so recover it here instead
    // of threading another parameter through.
    let licensing_note = message
        .as_deref()
        .filter(|note| note.starts_with("MangaDex licensing:"))
        .map(str::to_string);
    let with_licensing = |reason: String| match licensing_note.clone() {
        Some(note) => Some(format!("{note} {reason}")),
        None => Some(reason),
    };
    if let Some(reason) = rejection {
        status = TaskStatus::Failed;
        message = with_licensing(reason);
    } else if let Some(reason) = incomplete {
        status = TaskStatus::Failed;
        message = with_licensing(reason);
    } else if let Some(reason) = export_failed {
        status = TaskStatus::Failed;
        message = with_licensing(reason);
    } else if status == TaskStatus::Failed {
        // Keep the process failure reason.
    } else if outputs.is_empty() {
        status = TaskStatus::Failed;
        message = Some(zero_output_message(message.take()));
    } else {
        status = TaskStatus::Completed;
        // A successful job usually has no message. Manga keeps the licensing
        // note so a partial CBZ is not presented as a complete one.
    }

    let task = update_task(&state, &job_id, |task| {
        task.status = status;
        task.phase = if status == TaskStatus::Completed {
            TaskPhase::Completed
        } else {
            task.phase
        };
        task.progress = if status == TaskStatus::Completed {
            100.0
        } else {
            task.progress.min(99.0)
        };
        task.speed_bps = 0.0;
        task.eta_seconds = None;
        task.idle_seconds = 0;
        task.message = message.clone();
        task.files = outputs;
    });

    if let Some(task) = task {
        if status == TaskStatus::Completed {
            log_system(
                &app,
                &job_id,
                format!("Success — {} verified output file(s).", task.files.len()),
            );
        } else {
            log_stderr(
                &app,
                &job_id,
                format!(
                    "Failure — {}",
                    task.message.as_deref().unwrap_or("unknown error")
                ),
            );
        }
        emit_task(&app, &task);
        let _ = app.emit(EVENT_LIBRARY, now_ms());
    }

    status
}

#[allow(clippy::too_many_arguments)]
async fn process_epub_download(
    app: &AppHandle,
    state: &AppState,
    job_id: &str,
    destination: &Path,
    settings: &Settings,
    source_url: &str,
    options: &DownloadOptions,
    engine: Engine,
    primary_path: &Path,
    outputs: &mut Vec<OutputFile>,
    incomplete: &mut Option<String>,
    export_failed: &mut Option<String>,
    timings: &mut JobTimings,
    work_dir: Option<&Path>,
    progress: Arc<JobProgress>,
) {
    if primary_path
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|value| !value.eq_ignore_ascii_case("epub"))
    {
        let report = verify::verify_any(primary_path, false);
        if report.ok {
            outputs.push(output_file(primary_path, Some(report)));
        } else {
            let _ = std::fs::remove_file(primary_path);
            *export_failed = Some(format!(
                "Rejected {}: {}",
                file_name(primary_path),
                report.detail
            ));
        }
        return;
    }

    log_system(
        app,
        job_id,
        "Fetching real source metadata for the export manifest",
    );
    set_phase(state, app, job_id, TaskPhase::FetchingIndex);
    let metadata_timeout = Duration::from_secs(30);
    let index_timeout = Duration::from_secs(300);
    let metadata_started = Instant::now();
    let metadata_result =
        tokio::time::timeout(metadata_timeout, metadata::fetch_novel_metadata(source_url)).await;
    JobTimings::add(&mut timings.metadata, metadata_started.elapsed());
    let index_started = Instant::now();
    let index_result = tokio::time::timeout(
        index_timeout,
        metadata::fetch_chapter_index_for(settings, source_url, false),
    )
    .await;
    JobTimings::add(&mut timings.index, index_started.elapsed());
    log_system(
        app,
        job_id,
        format!(
            "Source transport · {}",
            transport::summary_for_url(source_url)
        ),
    );

    let metadata = match metadata_result {
        Ok(Ok(value)) => {
            log_system(
                app,
                job_id,
                format!(
                    "Metadata · {} · {} · {} chapter(s)",
                    value.title,
                    value.author.as_deref().unwrap_or("unknown author"),
                    value
                        .chapter_count
                        .map(|count| count.to_string())
                        .unwrap_or_else(|| "?".to_string())
                ),
            );
            if !value.title.trim().is_empty() {
                update_task(state, job_id, |task| task.title = value.title.clone());
            }
            Some(value)
        }
        Ok(Err(error)) => {
            log_system(
                app,
                job_id,
                format!("Metadata unavailable ({error}); using EPUB package data"),
            );
            None
        }
        Err(_) => {
            log_system(
                app,
                job_id,
                "Metadata request timed out; using EPUB package data",
            );
            None
        }
    };
    let mut index_problem: Option<String> = None;
    let index = match index_result {
        Ok(Ok(value)) => {
            let mut value = value;
            let cache_note = if value.cached {
                value
                    .cache_age_seconds
                    .map(|age| format!(" · cached {age}s ago"))
                    .unwrap_or_else(|| " · cached".to_string())
            } else {
                String::new()
            };
            log_system(
                app,
                job_id,
                format!(
                    "Chapter index · {} chapter(s) from {}{}{}",
                    value.total,
                    value.source,
                    cache_note,
                    value
                        .detail
                        .as_deref()
                        .map(|detail| format!(" · {detail}"))
                        .unwrap_or_default()
                ),
            );

            // The index carries its own evidence. A failed completeness proof
            // is a job failure even when the metadata page had no chapter count.
            if let Some(expected) = metadata.as_ref().and_then(|item| item.chapter_count)
                && value.total < expected
            {
                value.complete = false;
                let reason = format!(
                    "listed {} of {} chapters advertised by the source metadata",
                    value.total, expected
                );
                value.detail = Some(match value.detail.take() {
                    Some(existing) => format!("{existing}; {reason}"),
                    None => reason,
                });
            }

            if !value.complete {
                let reason = value.detail.as_deref().unwrap_or(
                    "the source did not provide enough evidence to prove the list is complete",
                );
                index_problem = Some(format!(
                    "Chapter index could not be proven complete: {reason}. Retry the download."
                ));
                log_stderr(app, job_id, index_problem.clone().unwrap_or_default());
            }
            Some(value)
        }
        Ok(Err(error)) => {
            index_problem = Some(format!(
                "Full chapter index could not be fetched: {error}. Retry the download."
            ));
            log_stderr(app, job_id, index_problem.clone().unwrap_or_default());
            None
        }
        Err(_) => {
            index_problem = Some(
                "Full chapter index timed out. Retry the download to fetch it in full.".to_string(),
            );
            log_stderr(app, job_id, index_problem.clone().unwrap_or_default());
            None
        }
    };

    let mut epub_path = primary_path.to_path_buf();
    set_phase(state, app, job_id, TaskPhase::Auditing);
    let audit_started = Instant::now();
    let mut audit = match export::audit_epub(&epub_path) {
        Ok(value) => value,
        Err(error) => {
            *export_failed = Some(format!("Could not audit EPUB: {error}"));
            return;
        }
    };
    JobTimings::add(&mut timings.audit, audit_started.elapsed());
    let mut plan = export::plan_audit(
        &audit,
        options.chapters.as_deref(),
        metadata.as_ref().and_then(|value| value.chapter_count),
        index.as_ref(),
    );
    log_system(app, job_id, format!("Chapter audit · {}", plan.detail));

    if engine == Engine::Webnovel && !plan.ok {
        set_phase(state, app, job_id, TaskPhase::Retrying);
        let retry_started = Instant::now();
        for attempt in 1..=2 {
            let Some((mode, spec)) =
                retry_instruction(&plan, options.chapters.as_deref(), index.as_ref())
            else {
                break;
            };
            let label = match mode {
                RetryMode::Resume => "resuming missing chapters",
                RetryMode::Refresh => "re-fetching empty chapters",
            };
            // The retry spec is built from the index's visible numbers, so it
            // needs the same serial translation as the first run (NF-044).
            let engine_spec = match concrete_selection(spec.as_deref()) {
                Some(selection) => {
                    let Some(index) = index.as_ref() else {
                        log_stderr(
                            app,
                            job_id,
                            "Cannot retry the selected chapters: the source chapter list is unavailable. Keeping the partial output.",
                        );
                        break;
                    };
                    match engine_chapter_spec(index, selection) {
                        Ok(mapped) => mapped,
                        Err(error) => {
                            log_stderr(
                                app,
                                job_id,
                                format!("Cannot retry the selected chapters: {error}"),
                            );
                            break;
                        }
                    }
                }
                None => None,
            };
            log_system(
                app,
                job_id,
                format!(
                    "Retry {attempt}/2 · {label} · {}",
                    engine_spec.as_deref().unwrap_or("all")
                ),
            );
            update_task(state, job_id, |task| {
                task.message = Some(format!("Retrying incomplete chapters ({attempt}/2)…"));
            });

            match run_lncrawl_retry(
                app,
                job_id,
                settings,
                destination,
                source_url,
                options,
                mode,
                engine_spec.as_deref(),
                work_dir,
                Arc::clone(&progress),
            )
            .await
            {
                Ok(Some(new_path)) => {
                    epub_path = new_path;
                    let audit_started = Instant::now();
                    audit = match export::audit_epub(&epub_path) {
                        Ok(value) => value,
                        Err(error) => {
                            log_stderr(app, job_id, format!("Retry EPUB audit failed: {error}"));
                            break;
                        }
                    };
                    JobTimings::add(&mut timings.audit, audit_started.elapsed());
                    plan = export::plan_audit(
                        &audit,
                        options.chapters.as_deref(),
                        metadata.as_ref().and_then(|value| value.chapter_count),
                        index.as_ref(),
                    );
                    log_system(app, job_id, format!("Retry audit · {}", plan.detail));
                    if plan.ok {
                        break;
                    }
                }
                Ok(None) => {
                    log_system(
                        app,
                        job_id,
                        "Retry produced no new artifact; keeping the partial output",
                    );
                    break;
                }
                Err(error) => {
                    log_stderr(app, job_id, format!("Retry failed: {error}"));
                    break;
                }
            }
        }
        JobTimings::add(&mut timings.retry, retry_started.elapsed());
        set_phase(state, app, job_id, TaskPhase::Auditing);
    }

    if audit.chapters.is_empty() {
        plan.ok = false;
        plan.found = 0;
        plan.detail = "No chapter files were found in the EPUB".to_string();
    }

    let fallback = metadata.unwrap_or_else(|| NovelMetadata {
        title: title_from_epub_or_path(&audit, &epub_path),
        source_url: source_url.to_string(),
        source: source_url.to_string(),
        ..Default::default()
    });
    let verification = export::verification_from_plan(&plan, audit.has_cover);
    set_phase(state, app, job_id, TaskPhase::Exporting);
    if let Some(previous) = export::read_export_metadata(destination) {
        let downloaded = previous
            .chapters
            .iter()
            .filter(|chapter| chapter.downloaded)
            .count();
        if downloaded > 0 {
            log_system(
                app,
                job_id,
                format!(
                    "Existing structured folder · preserving up to {downloaded} downloaded chapter(s) not in this run"
                ),
            );
        }
    }
    let export_started = Instant::now();
    match export::export_epub(
        &epub_path,
        destination,
        &fallback,
        index.as_ref(),
        options.chapters.as_deref(),
        &verification,
        true,
    ) {
        Ok(exported) => {
            log_system(
                app,
                job_id,
                format!(
                    "Structured export · metadata.json + index.json + chapters.html · {} chapter file(s) · {}",
                    exported.downloaded_chapters,
                    exported.cover.as_deref().unwrap_or("no cover")
                ),
            );
            // NF-041: persist the real crawl cost for the Library. An update's
            // export merges previously downloaded chapters, so the export count
            // is the whole book; only the selected chapters were crawled this
            // run. A resumed crawl adds the retry leg so the pace reflects the
            // whole run.
            let crawled =
                crawled_chapter_count(options.chapters.as_deref(), exported.downloaded_chapters);
            let crawl = timings.crawl.unwrap_or_default() + timings.retry.unwrap_or_default();
            if let Some(pace) = export::record_crawl_pace(destination, crawled, crawl) {
                log_system(
                    app,
                    job_id,
                    format!(
                        "Measured crawl pace · {:.1} chapters/min · {:.2} s/chapter · {} chapter(s)",
                        pace.chapters_per_minute, pace.seconds_per_chapter, pace.chapters
                    ),
                );
            }
        }
        Err(error) => {
            *export_failed = Some(format!("Structured export failed: {error}"));
            log_stderr(app, job_id, format!("Structured export failed: {error}"));
        }
    }
    JobTimings::add(&mut timings.export, export_started.elapsed());

    if let Some(problem) = index_problem
        && export_failed.is_none()
    {
        *export_failed = Some(problem);
    }

    set_phase(state, app, job_id, TaskPhase::Verifying);
    let verify_started = Instant::now();
    let mut report = verify::verify_any(&epub_path, engine == Engine::Books);
    report.chapters_expected = plan.expected;
    report.chapters_found = plan.found;
    report.empty_chapters = plan.empty_positions.clone();
    report.missing_chapters = plan.missing_serials.clone();
    report.has_cover = audit.has_cover;
    report.ok = report.ok && plan.ok;
    report.detail = format!("{} · {}", report.detail, plan.detail);
    outputs.push(output_file(&epub_path, Some(report)));
    JobTimings::add(&mut timings.verify, verify_started.elapsed());

    if !plan.ok {
        *incomplete = Some(format!("Incomplete output — {}", plan.detail));
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_book_download(
    app: &AppHandle,
    state: &AppState,
    job_id: &str,
    destination: &Path,
    engine: Engine,
    options: &DownloadOptions,
    primary_path: &Path,
    outputs: &mut Vec<OutputFile>,
    incomplete: &mut Option<String>,
    export_failed: &mut Option<String>,
    timings: &mut JobTimings,
) {
    if !primary_path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("epub"))
    {
        let report = verify::verify_any(primary_path, engine == Engine::Books);
        if report.ok {
            outputs.push(output_file(primary_path, Some(report)));
        } else {
            let _ = std::fs::remove_file(primary_path);
            *export_failed = Some(format!(
                "Rejected {}: {}",
                file_name(primary_path),
                report.detail
            ));
        }
        return;
    }

    let audit = match export::audit_epub(primary_path) {
        Ok(value) => value,
        Err(error) => {
            *export_failed = Some(format!("Could not audit book EPUB: {error}"));
            return;
        }
    };
    let plan = export::plan_audit(&audit, None, None, None);
    let verification = export::verification_from_plan(&plan, audit.has_cover);
    let title = audit
        .title
        .clone()
        .unwrap_or_else(|| file_name(primary_path));

    // Series books are shelved as `<NN> - <Title>.epub` inside the per-book
    // folder. Everything below (export, verification, outputs) uses the final
    // path so the library and the task never point at the old name.
    let mut epub_path = primary_path.to_path_buf();
    if options
        .book_series
        .as_deref()
        .is_some_and(|series| !series.trim().is_empty())
    {
        let target = destination.join(format!("{}.epub", series_book_stem(options, &title)));
        if target != epub_path {
            match std::fs::rename(&epub_path, &target) {
                Ok(()) => {
                    epub_path = target;
                    log_system(app, job_id, format!("Shelved as {}", file_name(&epub_path)));
                }
                Err(error) => {
                    log_stderr(
                        app,
                        job_id,
                        format!("Could not rename the EPUB to the series layout: {error}"),
                    );
                }
            }
        }
    }

    set_phase(state, app, job_id, TaskPhase::Exporting);
    let export_started = Instant::now();
    if let Err(error) = export::export_book_epub(&epub_path, destination, &title) {
        *export_failed = Some(format!("Structured book export failed: {error}"));
        log_stderr(
            app,
            job_id,
            format!("Structured book export failed: {error}"),
        );
    } else {
        log_system(
            app,
            job_id,
            format!(
                "Structured book export · metadata.json · {} chapter file(s)",
                plan.found
            ),
        );
    }
    JobTimings::add(&mut timings.export, export_started.elapsed());

    set_phase(state, app, job_id, TaskPhase::Verifying);
    let verify_started = Instant::now();
    let mut report = verify::verify_any(&epub_path, engine == Engine::Books);
    report.chapters_found = plan.found;
    report.empty_chapters = plan.empty_positions.clone();
    report.has_cover = audit.has_cover;
    report.ok = report.ok && plan.ok && plan.found > 0;
    report.detail = format!("{} · {}", report.detail, plan.detail);
    let complete = report.ok;
    outputs.push(output_file(&epub_path, Some(report)));
    JobTimings::add(&mut timings.verify, verify_started.elapsed());

    if !complete {
        *incomplete = Some(format!("Incomplete book output — {}", plan.detail));
    }
    let _ = verification;
}

#[allow(clippy::too_many_arguments)]
async fn process_manga_download(
    app: &AppHandle,
    state: &AppState,
    job_id: &str,
    destination: &Path,
    source_url: &str,
    options: &DownloadOptions,
    primary_path: &Path,
    title: &str,
    work_dir: Option<&Path>,
    outputs: &mut Vec<OutputFile>,
    incomplete: &mut Option<String>,
    export_failed: &mut Option<String>,
    timings: &mut JobTimings,
) {
    let extension = primary_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "cbz" | "zip") {
        let report = verify::verify_any(primary_path, false);
        if report.ok {
            outputs.push(output_file(primary_path, Some(report)));
        } else {
            let _ = std::fs::remove_file(primary_path);
            *export_failed = Some(format!(
                "Rejected {}: {}",
                file_name(primary_path),
                report.detail
            ));
        }
        return;
    }

    let audit = match export::audit_cbz(primary_path) {
        Ok(value) => value,
        Err(error) => {
            *export_failed = Some(format!("Could not audit manga archive: {error}"));
            return;
        }
    };
    let expected = options
        .chapters
        .as_deref()
        .and_then(export::parse_selection_serials)
        .map(|values| values.len() as u32);
    let empty = audit
        .chapters
        .iter()
        .filter(|chapter| chapter.images.is_empty())
        .count();
    let ok = !audit.chapters.is_empty()
        && empty == 0
        && expected.is_none_or(|value| audit.chapters.len() as u32 >= value);
    let detail = if ok {
        format!(
            "{} chapter folder(s) · {} page(s)",
            audit.chapters.len(),
            audit.image_count
        )
    } else {
        format!(
            "{} chapter folder(s), {} expected · {} empty · {} page(s)",
            audit.chapters.len(),
            expected
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string()),
            empty,
            audit.image_count
        )
    };
    log_system(app, job_id, format!("Manga audit · {detail}"));

    log_system(app, job_id, "Reading gallery-dl chapter metadata");
    let metadata = work_dir
        .and_then(gallery_dl_metadata)
        .map(|mut metadata| {
            metadata.source_url = source_url.to_string();
            metadata.source = Url::parse(source_url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_string))
                .unwrap_or_else(|| metadata.source.clone());
            metadata.title = non_blank(&metadata.title).unwrap_or_else(|| title.to_string());
            metadata
        })
        .unwrap_or_else(|| NovelMetadata {
            title: title.to_string(),
            source_url: source_url.to_string(),
            source: source_url.to_string(),
            tags: vec!["Manga".to_string()],
            ..Default::default()
        });

    let verification = ExportVerification {
        ok,
        chapters_expected: expected,
        chapters_found: audit.chapters.len() as u32,
        empty_chapters: Vec::new(),
        missing_chapters: Vec::new(),
        has_cover: audit.cover_bytes.is_some(),
        detail: detail.clone(),
    };
    set_phase(state, app, job_id, TaskPhase::Exporting);
    let export_started = Instant::now();
    if let Err(error) = export::export_cbz(
        primary_path,
        destination,
        &metadata,
        options.chapters.as_deref(),
        &verification,
    ) {
        *export_failed = Some(format!("Structured manga export failed: {error}"));
        log_stderr(
            app,
            job_id,
            format!("Structured manga export failed: {error}"),
        );
    } else {
        log_system(
            app,
            job_id,
            "Structured manga export · image folders · metadata.json",
        );
    }
    JobTimings::add(&mut timings.export, export_started.elapsed());

    set_phase(state, app, job_id, TaskPhase::Verifying);
    let verify_started = Instant::now();
    let mut report = verify::verify_any(primary_path, false);
    report.chapters_expected = expected;
    report.chapters_found = audit.chapters.len() as u32;
    report.has_cover = audit.cover_bytes.is_some();
    report.ok = report.ok && ok;
    report.detail = format!("{} · {}", report.detail, detail);
    outputs.push(output_file(primary_path, Some(report)));
    JobTimings::add(&mut timings.verify, verify_started.elapsed());

    if !ok {
        *incomplete = Some(format!("Incomplete manga output — {detail}"));
    }
}

#[derive(Debug, Clone, Copy)]
enum RetryMode {
    Resume,
    Refresh,
}

fn retry_instruction(
    plan: &export::AuditPlan,
    selection: Option<&str>,
    _index: Option<&NovelChapterIndex>,
) -> Option<(RetryMode, Option<String>)> {
    if !plan.empty_serials.is_empty() {
        return Some((
            RetryMode::Refresh,
            Some(format_serials(&plan.empty_serials)),
        ));
    }
    if !plan.empty_positions.is_empty() {
        return Some((RetryMode::Refresh, selection.map(str::to_string)));
    }
    if !plan.missing_serials.is_empty() {
        return Some((
            RetryMode::Resume,
            Some(format_serials(&plan.missing_serials)),
        ));
    }
    if plan.expected.is_some_and(|expected| plan.found < expected) {
        return Some((RetryMode::Resume, selection.map(str::to_string)));
    }
    None
}

fn format_serials(values: &[u32]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts = Vec::new();
    let mut index = 0;
    while index < sorted.len() {
        let start = sorted[index];
        let mut end = start;
        while index + 1 < sorted.len() && sorted[index + 1] == end + 1 {
            index += 1;
            end = sorted[index];
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
        index += 1;
    }
    parts.join(",")
}

#[allow(clippy::too_many_arguments)]
async fn run_lncrawl_retry(
    app: &AppHandle,
    job_id: &str,
    settings: &Settings,
    destination: &Path,
    source_url: &str,
    options: &DownloadOptions,
    mode: RetryMode,
    spec: Option<&str>,
    work_dir: Option<&Path>,
    progress: Arc<JobProgress>,
) -> Result<Option<PathBuf>, String> {
    let tool = resolve_tool(settings, Engine::Webnovel)
        .ok_or_else(|| "lncrawl is not available for the retry".to_string())?;
    let started = SystemTime::now();
    let mut args = vec![
        "-ll".to_string(),
        "crawl".to_string(),
        source_url.to_string(),
        "-f".to_string(),
        "epub".to_string(),
        "--noin".to_string(),
    ];
    args.extend(webnovel_rate_limit_args(options, source_url));
    match mode {
        RetryMode::Resume => args.push("--resume".to_string()),
        RetryMode::Refresh => args.push("--refresh".to_string()),
    }
    match concrete_selection(spec) {
        Some(value) => {
            args.push("--chapters".to_string());
            args.push(value.to_string());
        }
        None => args.push("--all".to_string()),
    }

    let mut command = Command::new(spawn::native_path(&tool.program));
    command
        .args(&tool.prefix)
        .args(&args)
        .current_dir(spawn::native_path(destination))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .env("TERM", "dumb")
        .env("COLUMNS", "240")
        .env("PYTHONUNBUFFERED", "1");
    if let Some(directory) = work_dir {
        lncrawl_data_env(&mut command, directory);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    spawn::hide_console(&mut command);

    log_system(app, job_id, format!("$ {}", tool.command_line(&args)));
    progress.reset_transfer();
    progress.set_phase(TaskPhase::Retrying);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start retry: {error}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_handle = stdout.map(|handle| {
        tokio::spawn(pump_lines(
            handle,
            LogStream::Stdout,
            app.clone(),
            job_id.to_string(),
            Arc::clone(&progress),
            None,
            None,
        ))
    });
    let stderr_handle = stderr.map(|handle| {
        tokio::spawn(pump_lines(
            handle,
            LogStream::Stderr,
            app.clone(),
            job_id.to_string(),
            Arc::clone(&progress),
            None,
            None,
        ))
    });

    let status = match tokio::time::timeout(Duration::from_secs(1_800), child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            return Err(format!("Retry supervision failed: {error}"));
        }
        Err(_) => {
            let _ = child.start_kill();
            return Err("Retry timed out after 30 minutes".to_string());
        }
    };
    if let Some(handle) = stdout_handle {
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
    }
    if let Some(handle) = stderr_handle {
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
    }
    if !status.success() {
        return Err(format!(
            "Retry exited with code {}",
            status
                .code()
                .map_or_else(|| "signal".to_string(), |value| value.to_string())
        ));
    }

    let source = recent_lncrawl_artifacts(started, work_dir)
        .into_iter()
        .find(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("epub"))
        });
    let Some(source) = source else {
        return Ok(None);
    };
    let name = source
        .file_name()
        .map(|value| value.to_owned())
        .ok_or_else(|| "Retry artifact has no filename".to_string())?;
    let target = unique_target(destination, &name);
    std::fs::rename(&source, &target)
        .or_else(|_| std::fs::copy(&source, &target).map(|_| ()))
        .map_err(|error| format!("Could not import retry artifact: {error}"))?;
    log_system(
        app,
        job_id,
        format!("Imported retry artifact {}", file_name(&target)),
    );
    Ok(Some(target))
}

fn choose_primary(scanned: &[PathBuf], engine: Engine) -> Option<PathBuf> {
    let preferred: &[&str] = match engine {
        Engine::Webnovel => &["epub"],
        Engine::Manga => &["cbz", "zip"],
        Engine::Books => &["epub", "pdf", "mobi", "azw3", "txt"],
    };
    for extension in preferred {
        let mut candidates = scanned
            .iter()
            .filter(|path| {
                path.extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case(extension))
            })
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|value| value.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        });
        if let Some(path) = candidates.pop() {
            return Some(path);
        }
    }
    scanned
        .iter()
        .max_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|value| value.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        })
        .cloned()
}

fn title_from_epub_or_path(audit: &export::EpubAudit, path: &Path) -> String {
    audit
        .title
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "Download".to_string())
        })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

async fn terminate(_child: &mut Child, pid: u32) {
    #[cfg(unix)]
    {
        signal_group(pid, libc::SIGCONT);
        signal_group(pid, libc::SIGTERM);
        tokio::time::sleep(Duration::from_millis(900)).await;
        signal_group(pid, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        // `taskkill /T` is the only way to reach the Python grandchildren that
        // the venv launcher creates; killing the launcher alone orphans them.
        terminate_process(pid);
        tokio::time::sleep(Duration::from_millis(900)).await;
        force_kill_process(pid);
        let _ = _child.start_kill();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        let _ = _child.start_kill();
    }
}

/// Last stderr lines of a child process, kept so a failed crawl reports the
/// engine's own reason instead of only its exit code.
#[derive(Default)]
struct ErrorTail {
    lines: VecDeque<String>,
    /// MangaDex reports one external-only line per chapter, which can exceed the
    /// ring buffer. Counting happens as lines arrive so a 200-chapter series is
    /// summarised correctly even though only the last 24 lines are kept.
    external_chapters: BTreeSet<String>,
    external_url: Option<String>,
    external_hosts: BTreeSet<String>,
}

/// Enough to cover a traceback plus the engine's final `Exhausted:` block.
const ERROR_TAIL_LINES: usize = 24;

/// NF-042: an actionable line on lncrawl's stdout. The traceback header and
/// `File "…"` frames are deliberately skipped; the final exception line is the
/// one a user can act on. INFO/WARNING rows never qualify.
fn stdout_carries_reason(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("traceback")
        || lower.starts_with("file \"")
        || lower.starts_with("during handling")
        || lower.starts_with("the above exception")
    {
        return false;
    }
    if lower.starts_with('[')
        && (lower.contains(" info ") || lower.contains(" warning ") || lower.contains(" debug "))
    {
        return false;
    }
    lower.starts_with("exhausted:")
        || lower.starts_with("error:")
        || lower.starts_with("error ")
        || lower.starts_with("httperror")
        || lower.starts_with("unsupported url")
        || lower.starts_with("invalid url")
        || lower.starts_with("no crawler")
        || lower.contains("servererror")
        || lower.contains("no crawler found")
        || lower.contains("exception:")
        || lower.contains("connectionerror")
        || lower.contains("timeouterror")
        || lower.contains("sslerror")
}

fn mangadex_external_line(line: &str) -> Option<(String, String, String)> {
    use std::sync::OnceLock;

    // gallery-dl: `[mangadex][error] Chapter 1 is not available on MangaDex and
    // can instead be read on the official publisher's website at https://…`
    static EXTERNAL: OnceLock<Regex> = OnceLock::new();
    let pattern = EXTERNAL.get_or_init(|| {
        Regex::new(
            r"(?i)chapter\s+([0-9]+(?:\.[0-9]+)?)\s+is not available on mangadex.*?(https?://([^/\s]+)\S*)",
        )
        .expect("valid MangaDex licensing regex")
    });
    let captures = pattern.captures(line)?;
    let url = captures
        .get(2)?
        .as_str()
        .trim_end_matches(['.', ',', ';', ')'])
        .to_string();
    Some((
        captures.get(1)?.as_str().to_string(),
        url,
        captures.get(3)?.as_str().to_string(),
    ))
}

impl ErrorTail {
    fn push(&mut self, line: &str) {
        if let Some((chapter, url, host)) = mangadex_external_line(line) {
            self.external_chapters.insert(chapter);
            self.external_hosts.insert(host);
            self.external_url.get_or_insert(url);
        }
        if self.lines.len() == ERROR_TAIL_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line.to_string());
    }

    /// NF-042: lncrawl reports fatal crawl errors and tracebacks on **stdout**
    /// and then exits 0. Keep only the lines that carry a reason, so a normal
    /// progress stream cannot displace the final exception line.
    fn push_stdout(&mut self, line: &str) {
        if !stdout_carries_reason(line) {
            return;
        }
        if self.lines.len() == ERROR_TAIL_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line.to_string());
    }

    /// A user-facing explanation for MangaDex's licensing gap, when gallery-dl
    /// reported one. The publisher page is never scraped; this only names it.
    /// Compact, user-facing explanation for MangaDex's licensing gap. This lands
    /// in the job row, so it names the chapter and the publisher host rather
    /// than pasting three long URLs into a table cell.
    fn licensing_note(&self) -> Option<String> {
        let count = self.external_chapters.len();
        if count == 0 {
            return None;
        }
        let reading = self.reading_site();
        let subject = if count == 1 {
            format!(
                "chapter {} is",
                self.external_chapters
                    .iter()
                    .next()
                    .map(String::as_str)
                    .unwrap_or("1")
            )
        } else {
            format!("{count} chapters are")
        };
        Some(format!(
            "MangaDex licensing: {subject} external-only — read at {reading}. \
The publisher page is never scraped."
        ))
    }

    /// Full explanation with the publisher URLs, logged to the dock.
    fn licensing_detail(&self) -> Option<String> {
        let count = self.external_chapters.len();
        if count == 0 {
            return None;
        }
        let reading = match &self.external_url {
            Some(url) => url.clone(),
            None => "the publisher's site".to_string(),
        };
        let primary_host = self
            .external_url
            .as_deref()
            .and_then(|url| Url::parse(url).ok())
            .and_then(|url| url.host_str().map(str::to_string));
        let others = self
            .external_hosts
            .iter()
            .filter(|host| Some(host.as_str()) != primary_host.as_deref())
            .collect::<Vec<_>>();
        let extra = if others.is_empty() {
            String::new()
        } else {
            format!(
                " ({} other publisher edition(s): {})",
                others.len(),
                others
                    .iter()
                    .take(2)
                    .map(|host| host.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        Some(format!(
            "MangaDex licensing: {} external-only. First publisher link: \
{reading}{extra}. NovelForge never scrapes the publisher page and keeps any chapters \
that were available. Try another extractor (for example a Weebcentral URL) if the \
series is available there.",
            if count == 1 {
                format!(
                    "chapter {} is",
                    self.external_chapters
                        .iter()
                        .next()
                        .map(String::as_str)
                        .unwrap_or("1")
                )
            } else {
                format!("{count} chapters are")
            }
        ))
    }

    /// Short display form for the site list: one host, or host + count.
    fn reading_site(&self) -> String {
        // Prefer the host of the first link seen so the note and the logged
        // detail agree on which publisher edition they point at.
        let primary = self
            .external_url
            .as_deref()
            .and_then(|url| Url::parse(url).ok())
            .and_then(|url| url.host_str().map(str::to_string))
            .or_else(|| self.external_hosts.iter().next().cloned());
        match primary {
            None => "the publisher's site".to_string(),
            Some(host) if self.external_hosts.len() <= 1 => host,
            Some(host) => format!(
                "{host} and {} other publisher(s)",
                self.external_hosts.len() - 1
            ),
        }
    }

    /// lncrawl prints the actionable failure first (`Exhausted:` / `HTTPError` /
    /// `Traceback`) and then multi-line retry notes. Prefer that block, then the
    /// newest line that reads like an error, then simply the last line.
    fn summary(&self) -> Option<String> {
        let is_headline = |line: &str| {
            let lower = line.trim_start().to_ascii_lowercase();
            if lower.starts_with("traceback (most recent call last)") {
                // The header alone names no cause; the final exception line
                // that follows it is the actionable one.
                return false;
            }
            lower.starts_with("exhausted:")
                || lower.starts_with("httperror")
                || lower.starts_with("traceback")
                || lower.starts_with("error:")
                || lower.starts_with("error ")
                || lower.contains("servererror")
                || lower.contains("no crawler found")
        };
        let is_errorish = |line: &str| {
            let lower = line.to_ascii_lowercase();
            lower.contains("error")
                || lower.contains("exception")
                || lower.contains("failed")
                || lower.contains("timeout")
                || lower.contains("timed out")
        };

        let picked: Vec<&str> = match self.lines.iter().rposition(|line| is_headline(line)) {
            Some(index) => self
                .lines
                .iter()
                .skip(index)
                .take(4)
                .map(String::as_str)
                .collect(),
            None => match self.lines.iter().rposition(|line| is_errorish(line)) {
                Some(index) => self
                    .lines
                    .iter()
                    .skip(index)
                    .take(2)
                    .map(String::as_str)
                    .collect(),
                None => self
                    .lines
                    .iter()
                    .rev()
                    .take(1)
                    .map(String::as_str)
                    .collect(),
            },
        };

        let collapsed = picked
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if collapsed.is_empty() {
            return None;
        }
        Some(collapsed.chars().take(300).collect())
    }

    /// NF-035: only a failure that reads like a transport or rate-limit problem
    /// is retried automatically. Anything that names the URL itself as unusable
    /// — or an empty tail, where the cause is unknown — fails closed.
    fn is_transient(&self) -> bool {
        let Some(summary) = self.summary() else {
            return false;
        };
        let lower = summary.to_ascii_lowercase();

        const FATAL: [&str; 6] = [
            "unsupported url",
            "invalid url",
            "no crawler",
            "not found",
            "404",
            "no chapters",
        ];
        if FATAL.iter().any(|marker| lower.contains(marker)) {
            return false;
        }

        const TRANSIENT: [&str; 18] = [
            "timeout",
            "timed out",
            "transport",
            "connection",
            "connect",
            "reset",
            "temporarily",
            "unavailable",
            "exhausted",
            "no detection layer",
            "429",
            "503",
            "1015",
            "cloudflare",
            "dns",
            "network",
            "unreachable",
            "stalled",
        ];
        TRANSIENT.iter().any(|marker| lower.contains(marker))
    }
}

async fn pump_lines<R>(
    reader: R,
    stream: LogStream,
    app: AppHandle,
    job_id: String,
    progress: Arc<JobProgress>,
    artifacts: Option<Arc<Mutex<ArtifactCapture>>>,
    error_tail: Option<Arc<Mutex<ErrorTail>>>,
) where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(raw)) = lines.next_line().await {
        let clean = clean_line(&raw);
        if clean.is_empty() {
            continue;
        }

        progress.touch();

        if let Some(total) = parse_advertised_total(&clean) {
            if progress.chapters_total.load(Ordering::Relaxed) == 0 {
                progress.chapters_total.store(total, Ordering::Relaxed);
            }
            progress.set_phase(TaskPhase::DownloadingChapters);
        }

        if let Some(sink) = &artifacts {
            capture_artifact_line(&clean, sink);
        }

        if let Some(tail) = &error_tail
            && let Ok(mut tail) = tail.lock()
        {
            if stream == LogStream::Stdout {
                tail.push_stdout(&clean);
            } else {
                tail.push(&clean);
            }
        }

        if let Some(sample) = parse_progress_line(&clean) {
            progress.set_phase(sample.phase);
            if let Some(value) = sample.percent {
                progress.percent.store(
                    (value * 100.0).round().clamp(0.0, 9_900.0) as u32,
                    Ordering::Relaxed,
                );
            }
            if let Some((done, total)) = sample.counts {
                progress.chapters_done.store(done, Ordering::Relaxed);
                progress.chapters_total.store(total, Ordering::Relaxed);
            }
        }

        let _ = app.emit(
            EVENT_LOG,
            LogEvent {
                job_id: job_id.clone(),
                stream,
                line: clean,
                ts: now_ms(),
            },
        );
    }
}

struct ProgressSample {
    phase: TaskPhase,
    percent: Option<f32>,
    counts: Option<(u32, u32)>,
}

/// lncrawl's tqdm bars are the only honest source of chapter-level progress.
/// A line is only treated as progress when its prefix names a known stage, so
/// chapter text containing `%` or `12/40` cannot move the bar.
fn parse_progress_line(line: &str) -> Option<ProgressSample> {
    let trimmed = line.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    let phase = if lower.starts_with("chapters:") {
        TaskPhase::DownloadingChapters
    } else if lower.starts_with("images:") {
        TaskPhase::DownloadingImages
    } else if lower.starts_with("generating epub") || lower.starts_with("generating cbz") {
        TaskPhase::Packing
    } else if lower.starts_with("fetching novel details") {
        TaskPhase::FetchingDetails
    } else {
        return None;
    };

    let percent = parse_percent(trimmed);
    let counts = parse_ratio(trimmed);
    if percent.is_none()
        && counts.is_none()
        && phase != TaskPhase::Packing
        && phase != TaskPhase::FetchingDetails
    {
        return None;
    }
    Some(ProgressSample {
        phase,
        percent,
        counts,
    })
}

fn parse_ratio(line: &str) -> Option<(u32, u32)> {
    use std::sync::OnceLock;

    static RATIO: OnceLock<Regex> = OnceLock::new();
    let ratio_re =
        RATIO.get_or_init(|| Regex::new(r"(\d{1,7})\s*/\s*(\d{1,7})").expect("valid ratio regex"));
    let captures = ratio_re.captures(line)?;
    let done = captures.get(1)?.as_str().parse::<u32>().ok()?;
    let total = captures.get(2)?.as_str().parse::<u32>().ok()?;
    if total >= 2 && done <= total {
        Some((done, total))
    } else {
        None
    }
}

/// lncrawl prints the novel panel as `N volumes, M chapters` even when the
/// progress bars are disabled on a pipe. That total is the only advertised
/// chapter count available to the app before the final index fetch.
fn parse_advertised_total(line: &str) -> Option<u32> {
    use std::sync::OnceLock;

    static TOTAL: OnceLock<Regex> = OnceLock::new();
    let total_re = TOTAL.get_or_init(|| {
        Regex::new(r"(?i)\d+\s+volumes?,\s*(\d{1,7})\s+chapters?")
            .expect("valid chapter total regex")
    });
    total_re
        .captures(line)?
        .get(1)?
        .as_str()
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
}

#[allow(clippy::too_many_arguments)]
async fn progress_loop(
    app: AppHandle,
    state: AppState,
    job_id: String,
    destination: PathBuf,
    work_dir: Option<PathBuf>,
    engine: Engine,
    progress: Arc<JobProgress>,
    done: Arc<AtomicBool>,
    started: Instant,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(900));
    let mut byte_samples: VecDeque<(Instant, u64)> = VecDeque::new();
    let mut percent_samples: VecDeque<(Instant, f32)> = VecDeque::new();
    let mut measure_path: Option<PathBuf> = None;
    let mut paused_total = Duration::ZERO;
    let mut paused_at: Option<Instant> = None;
    let mut last_bytes = 0_u64;

    loop {
        interval.tick().await;
        if done.load(Ordering::Relaxed) {
            break;
        }

        let (status, task_progress, task_phase) = {
            let Ok(jobs) = state.jobs.lock() else {
                break;
            };
            let Some(record) = jobs.get(&job_id) else {
                break;
            };
            (record.task.status, record.task.progress, record.task.phase)
        };
        if status.is_terminal() {
            break;
        }

        // Measure the isolated engine store while crawling. lncrawl writes
        // under `novels/`; gallery-dl writes `gallery-dl/<category>/<title>/`.
        // Once the artifact is imported, measure the job sandbox where the
        // structured export is written.
        let measuring_work = work_dir.is_some() && !task_phase.measures_destination();
        let target = match &work_dir {
            Some(directory) if measuring_work => match engine {
                Engine::Manga => directory.join("gallery-dl"),
                _ => directory.join("novels"),
            },
            _ => destination.clone(),
        };
        if measure_path.as_ref() != Some(&target) {
            byte_samples.clear();
            percent_samples.clear();
            measure_path = Some(target.clone());
            last_bytes = 0;
        }

        let now = Instant::now();
        let stats = if measuring_work {
            work_stats(&target, engine)
        } else {
            WorkStats {
                bytes: dir_size(&target),
                ..WorkStats::default()
            }
        };
        let bytes = stats.bytes;
        if bytes > last_bytes {
            progress.touch();
        }
        last_bytes = bytes;

        // Crawl phases come from lncrawl output. Finalize phases are written
        // directly to the task, except while a retry is running and its own
        // progress bars should take over again. When tqdm is disabled on a
        // pipe, chapter files on disk are the honest progress signal.
        let mut phase = if status == TaskStatus::Running || task_phase == TaskPhase::Retrying {
            progress.phase()
        } else {
            task_phase
        };
        let total = progress.chapters_total.load(Ordering::Relaxed);
        if measuring_work && task_phase != TaskPhase::Retrying && total > 0 && stats.chapters > 0 {
            let inferred = if stats.chapters >= total {
                if stats.images > 0 {
                    TaskPhase::DownloadingImages
                } else {
                    TaskPhase::Packing
                }
            } else {
                TaskPhase::DownloadingChapters
            };
            if phase.is_crawl() && inferred.rank() > phase.rank() {
                phase = inferred;
            }
        }

        byte_samples.push_back((now, bytes));
        while byte_samples
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > Duration::from_secs(8))
        {
            byte_samples.pop_front();
        }

        if status == TaskStatus::Paused {
            if paused_at.is_none() {
                paused_at = Some(now);
            }
        } else if let Some(at) = paused_at.take() {
            paused_total += now.duration_since(at);
            byte_samples.clear();
            percent_samples.clear();
            progress.touch();
        }

        let speed = if status == TaskStatus::Paused || !phase.shows_speed() {
            0.0
        } else if let (Some((first_at, first_bytes)), Some((last_at, last_bytes))) =
            (byte_samples.front(), byte_samples.back())
        {
            let seconds = last_at.duration_since(*first_at).as_secs_f64();
            if seconds > 0.5 && last_bytes > first_bytes {
                (last_bytes - first_bytes) as f64 / seconds
            } else {
                0.0
            }
        } else {
            0.0
        };

        let raw_percent = progress.percent.load(Ordering::Relaxed) as f32 / 100.0;
        let file_percent = if measuring_work && total > 0 && stats.chapters > 0 {
            (stats.chapters as f32 / total as f32 * 100.0).min(99.0)
        } else {
            0.0
        };
        let percent_value = if status == TaskStatus::Running || task_phase == TaskPhase::Retrying {
            raw_percent.max(file_percent).min(99.0)
        } else {
            task_progress
        };

        if phase.is_transfer() {
            if percent_samples
                .back()
                .is_some_and(|(_, previous)| percent_value + 0.5 < *previous)
            {
                percent_samples.clear();
            }
            percent_samples.push_back((now, percent_value));
            while percent_samples
                .front()
                .is_some_and(|(at, _)| now.duration_since(*at) > Duration::from_secs(20))
            {
                percent_samples.pop_front();
            }
        } else {
            percent_samples.clear();
        }

        let eta = if status == TaskStatus::Paused || !phase.is_transfer() {
            None
        } else {
            eta_from_samples(&percent_samples, percent_value)
        };

        let elapsed_seconds = started.elapsed().saturating_sub(paused_total).as_secs();
        let idle_seconds = if status == TaskStatus::Paused {
            0
        } else {
            progress.idle_seconds()
        };

        let task = update_task(&state, &job_id, |task| {
            task.phase = phase;
            task.elapsed_seconds = elapsed_seconds;
            task.idle_seconds = idle_seconds;
            if status == TaskStatus::Running || task_phase == TaskPhase::Retrying {
                task.progress = percent_value;
            }
            task.bytes = bytes;
            task.speed_bps = speed;
            task.eta_seconds = eta;
        });

        if let Some(task) = task {
            let _ = app.emit(
                EVENT_PROGRESS,
                ProgressEvent {
                    job_id: job_id.clone(),
                    percent: task.progress,
                    bytes: task.bytes,
                    speed_bps: task.speed_bps,
                    eta_seconds: task.eta_seconds,
                    phase: task.phase,
                    elapsed_seconds: task.elapsed_seconds,
                    idle_seconds: task.idle_seconds,
                    status: task.status,
                },
            );
        }
    }
}

/// ETA from recent chapter/request percent movement. Flat or backwards
/// movement produces no estimate instead of a fabricated one.
fn eta_from_samples(samples: &VecDeque<(Instant, f32)>, percent: f32) -> Option<u64> {
    if !(1.0..99.0).contains(&percent) || samples.len() < 2 {
        return None;
    }
    let (first_at, first_percent) = samples.front()?;
    let (last_at, last_percent) = samples.back()?;
    let seconds = last_at.duration_since(*first_at).as_secs_f64();
    if seconds < 3.0 {
        return None;
    }
    let delta = last_percent - first_percent;
    if delta <= 0.5 {
        return None;
    }
    let rate = delta as f64 / seconds;
    let estimate = (100.0 - percent as f64) / rate;
    if estimate.is_finite() && estimate > 0.0 {
        Some((estimate.round() as u64).min(86_400))
    } else {
        None
    }
}

fn validate_request(request: &DownloadRequest) -> AppResult<()> {
    let query = request.query.trim();
    if query.chars().count() < 2 {
        return Err(AppError::msg("Enter a title, author, or valid URL"));
    }
    if query.chars().count() > 500 {
        return Err(AppError::msg("Input is too long (maximum 500 characters)"));
    }
    if query.chars().any(char::is_control) {
        return Err(AppError::msg("Input contains control characters"));
    }
    if let Some(title) = &request.title
        && (title.chars().count() > 140 || title.chars().any(char::is_control))
    {
        return Err(AppError::msg("Title must be 140 characters or fewer"));
    }

    match request.engine {
        Engine::Webnovel => {
            if query.chars().count() < 2 {
                return Err(AppError::msg("Enter a novel title, synonym, or page URL"));
            }
        }
        Engine::Manga => {
            let url = Url::parse(query).map_err(|_| {
                AppError::msg(
                    "Manga jobs require a direct http(s) link. Pick a supported source below.",
                )
            })?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(AppError::msg("Only http(s) URLs are supported"));
            }
        }
        Engine::Books => {
            if query.chars().count() < 2 {
                return Err(AppError::msg("Enter a book title or author"));
            }
            for (label, value) in [
                ("Book title", request.options.book_title.as_deref()),
                ("Book author", request.options.book_author.as_deref()),
            ] {
                if let Some(value) = value
                    && (value.chars().count() > 200 || value.chars().any(char::is_control))
                {
                    return Err(AppError::msg(format!(
                        "{label} must be 200 characters or fewer"
                    )));
                }
            }
            if let Some(isbn) = request.options.book_isbn.as_deref() {
                let compact: String = isbn
                    .chars()
                    .filter(|character| !character.is_whitespace() && *character != '-')
                    .collect();
                let valid = compact.len() == 10 || compact.len() == 13;
                if !valid
                    || !compact.chars().all(|character| {
                        character.is_ascii_digit() || character == 'X' || character == 'x'
                    })
                {
                    return Err(AppError::msg("ISBN must be 10 or 13 digits"));
                }
            }
        }
    }

    Ok(())
}

/// lncrawl's CLI has no per-run chapter-concurrency flag. `--rate-limit` is the
/// supported transport control, so the webnovel throttle toggle must use it.
/// The rate comes from the same source profile that paces app-level requests.
fn format_source_rate(rate: f32) -> String {
    if (rate - rate.round()).abs() < 0.05 {
        format!("{:.0}", rate)
    } else {
        format!("{:.1}", rate)
    }
}

fn webnovel_rate_limit_args(options: &DownloadOptions, source_url: &str) -> Vec<String> {
    if options.throttle.unwrap_or(true) {
        vec![
            "--rate-limit".to_string(),
            format_source_rate(transport::chapter_rate_for_url(source_url)),
        ]
    } else {
        Vec::new()
    }
}

fn webnovel_transport_summary(options: &DownloadOptions, source_url: &str) -> String {
    if options.throttle.unwrap_or(true) {
        format!(
            "source requests capped at {}/sec (lncrawl --rate-limit; {} profile)",
            format_source_rate(transport::chapter_rate_for_url(source_url)),
            transport::profile_label_for_url(source_url)
        )
    } else {
        "throttle off — lncrawl source default pacing".to_string()
    }
}

/// Filesystem-safe display segment for the series layout. It keeps the
/// catalogue spelling (so the library reads naturally) but removes path
/// separators, control characters and any leading dot, and caps the length.
fn safe_segment(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
            {
                ' '
            } else {
                character
            }
        })
        .collect();
    let collapsed = cleaned
        .split_whitespace()
        .filter(|token| token.chars().any(|character| character != '.'))
        .collect::<Vec<_>>()
        .join(" ");
    let collapsed = collapsed.trim_matches('.').trim();
    if collapsed.is_empty() {
        "Unknown".to_string()
    } else {
        collapsed.chars().take(80).collect()
    }
}

/// `1` → `01`, `3.5` → `03.5`, invalid/zero → no prefix. A book without a
/// catalogue position is shelved under its title instead of an invented number.
fn series_position_label(position: f64) -> Option<String> {
    if !position.is_finite() || position <= 0.0 {
        return None;
    }
    let rounded = position.round();
    if (position - rounded).abs() < 1e-9 {
        Some(format!("{:02}", rounded as u32))
    } else {
        Some(format!("{position:04.1}"))
    }
}

fn series_book_stem(options: &DownloadOptions, title: &str) -> String {
    match options.book_series_position.and_then(series_position_label) {
        Some(position) => format!("{position} - {}", safe_segment(title)),
        None => safe_segment(title),
    }
}

/// `<root>/<Author>/<Series>/<NN> - <Title>`. Only used when a catalogue series
/// name is present, so ordinary single-book downloads keep their id suffix.
fn book_series_destination(root: &Path, options: &DownloadOptions, title: &str) -> Option<PathBuf> {
    let series = options
        .book_series
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let author = options
        .book_author
        .as_deref()
        .map(|value| value.split(',').next().unwrap_or(value).trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("Unknown Author");
    Some(
        root.join(safe_segment(author))
            .join(safe_segment(series))
            .join(series_book_stem(options, title)),
    )
}

fn duplicate_copy_destination(root: &Path, title: &str) -> AppResult<PathBuf> {
    let slug = slugify(title);
    for edition in 2..=999_u32 {
        let candidate = root.join(format!("{slug}-copy-{edition}"));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::msg(
        "Could not allocate a copy folder after 998 editions",
    ))
}

fn folder_is_busy(state: &AppState, folder: &Path) -> bool {
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

fn build_args(
    request: &DownloadRequest,
    settings: &Settings,
    destination: &Path,
    chapter_index: Option<&NovelChapterIndex>,
) -> AppResult<Vec<String>> {
    let query = request.query.trim().to_string();
    let mut args = Vec::new();

    match request.engine {
        Engine::Webnovel => {
            args.push("-ll".to_string());
            args.push("crawl".to_string());
            args.push(query.clone());
            args.push("-f".to_string());
            args.push("epub".to_string());
            match normalize_chapters(request.options.chapters.as_deref())? {
                Some(spec) => {
                    // The spec is the numbers the source shows; lncrawl selects
                    // catalogue serials. Without the list there is no way to
                    // prove which row a number belongs to, so a partial run
                    // fails closed instead of guessing.
                    let index = chapter_index.ok_or_else(|| {
                        AppError::msg(
                            "The source chapter list is needed to select these chapters, but it was not fetched.",
                        )
                    })?;
                    let mapped = engine_chapter_spec(index, &spec)?.unwrap_or(spec);
                    args.push("--chapters".to_string());
                    args.push(mapped);
                }
                None => args.push("--all".to_string()),
            }
            // Prevent any interactive prompt from stalling the background job.
            args.push("--noin".to_string());
            // `jobs` is intentionally not read here: lncrawl's CLI cannot set
            // chapter concurrency for a single crawl. The UI hides that control
            // for webnovels instead of pretending it works.
            args.extend(webnovel_rate_limit_args(&request.options, &query));
        }
        Engine::Manga => {
            // gallery-dl extracts images; NovelForge packs the single archive
            // in `package_manga_directory` after the process exits.
            if let Some(spec) = normalize_chapters(request.options.chapters.as_deref())?
                && let Some(filter) = manga_chapter_filter(&spec)
            {
                args.push("--child-filter".to_string());
                args.push(filter);
            }
            args.push("--no-input".to_string());
            args.push("--no-colors".to_string());
            if let Some(language) = settings.manga_language.as_deref() {
                args.push("--option".to_string());
                args.push(format!("lang={language}"));
            }
            // gallery-dl is the metadata authority for manga: the source page
            // is often a JavaScript app, while the extractor already knows the
            // series title, author, tags and description.
            args.push("--write-info-json".to_string());
            let throttle = request.options.throttle.unwrap_or(settings.manga_throttle);
            if throttle {
                // gallery-dl has no job pool; the honest politeness control is
                // a pause between HTTP requests.
                args.push("--sleep-request".to_string());
                args.push("1.0".to_string());
            }
            args.push(query);
        }
        Engine::Books => {
            let source = request
                .options
                .book_source
                .as_deref()
                .unwrap_or("auto")
                .trim()
                .to_ascii_lowercase();
            // `gutenberg` and `standardebooks` are lawful public-domain
            // catalogues. They are never part of `auto`; the UI exposes them
            // as explicit choices, so the default order is unchanged.
            if !matches!(
                source.as_str(),
                "auto" | "libgen" | "welib" | "annas" | "ocean" | "gutenberg" | "standardebooks"
            ) {
                return Err(AppError::msg(
                    "Book source must be auto, libgen, welib, annas, ocean, \
gutenberg or standardebooks",
                ));
            }
            args.push("--format".to_string());
            args.push("epub".to_string());
            args.push("--auto".to_string());
            args.push("--source".to_string());
            args.push(source);
            args.push("--output".to_string());
            // aget is Python: hand it the path without the verbatim prefix for
            // the same reason lncrawl's SQLite URL needed one.
            args.push(
                spawn::native_path(destination)
                    .to_string_lossy()
                    .to_string(),
            );
            args.push("--headless".to_string());
            if let Some(title) = request
                .options
                .book_title
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                args.push("--title".to_string());
                args.push(title.to_string());
            }
            if let Some(author) = request
                .options
                .book_author
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                args.push("--author".to_string());
                args.push(author.to_string());
            }
            if let Some(isbn) = request
                .options
                .book_isbn
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                args.push("--isbn".to_string());
                args.push(isbn.to_string());
            }
            if let Some(language) = settings
                .book_language
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                args.push("--lang".to_string());
                args.push(language.to_string());
            }
            args.push(query);
        }
    }

    Ok(args)
}

/// Parses `1-20`, `19,31,82` and mixtures into a sorted, deduplicated and
/// compressed spec. `None` means every chapter.
fn normalize_chapters(value: Option<&str>) -> AppResult<Option<String>> {
    let Some(raw) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if raw.eq_ignore_ascii_case("all") {
        return Ok(None);
    }

    let mut numbers = Vec::new();
    for token in raw.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some((start, end)) = token.split_once('-') {
            let start = parse_chapter_number(start, token)?;
            let end = parse_chapter_number(end, token)?;
            let (low, high) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            if high - low > 20_000 {
                return Err(AppError::msg("Chapter range is too large"));
            }
            numbers.extend(low..=high);
        } else {
            numbers.push(parse_chapter_number(token, token)?);
        }
        if numbers.len() > 20_000 {
            return Err(AppError::msg("Chapter selection is too large"));
        }
    }

    if numbers.is_empty() {
        return Ok(None);
    }

    numbers.sort_unstable();
    numbers.dedup();

    let mut parts = Vec::new();
    let mut index = 0;
    while index < numbers.len() {
        let start = numbers[index];
        let mut end = start;
        while index + 1 < numbers.len() && numbers[index + 1] == end + 1 {
            index += 1;
            end = numbers[index];
        }
        if start == end {
            parts.push(start.to_string());
        } else {
            parts.push(format!("{start}-{end}"));
        }
        index += 1;
    }

    Ok(Some(parts.join(",")))
}

/// A concrete selection, or `None` when the run should cover every chapter.
fn concrete_selection(value: Option<&str>) -> Option<&str> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("all"))
}

/// NF-044: lncrawl filters `--chapters` by `Chapter.serial`, the row's 1-based
/// position in the catalogue it built. The picker's numbers are the source's
/// own chapter numbers, and they are not the same thing: NovelPhoenix's ORV
/// starts at visible *Chapter 0* (serial 1) and its table of contents skips a
/// number that only exists under a longer URL. Translate the selection through
/// the index the crawler will see, and refuse a number the index cannot account
/// for instead of quietly fetching the neighbouring chapter.
fn engine_chapter_spec(index: &NovelChapterIndex, selection: &str) -> AppResult<Option<String>> {
    let Some(numbers) = crate::export::parse_selection_serials(selection) else {
        return Ok(None);
    };
    if !index.complete {
        let detail = index
            .detail
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!(" {value}"))
            .unwrap_or_default();
        return Err(AppError::msg(format!(
            "The source chapter list is incomplete, so the selected chapters were not downloaded.{detail}"
        )));
    }
    if index.chapters.is_empty() {
        return Err(AppError::msg(
            "The source chapter list is empty, so the selected chapters were not downloaded.",
        ));
    }

    let mut serial_by_number = HashMap::<u32, u32>::new();
    for (offset, chapter) in index.chapters.iter().enumerate() {
        let serial = offset as u32 + 1;
        if serial_by_number.insert(chapter.number, serial).is_some() {
            return Err(AppError::msg(format!(
                "The source lists Chapter {} more than once, so the selected chapters cannot be mapped onto the crawler",
                chapter.number
            )));
        }
    }

    let mut serials = Vec::with_capacity(numbers.len());
    for number in numbers {
        let Some(serial) = serial_by_number.get(&number) else {
            return Err(AppError::msg(format!(
                "Chapter {number} is not in the source's chapter list, so the download was not started. The list shows {} chapter(s).",
                index.chapters.len()
            )));
        };
        serials.push(*serial);
    }
    serials.sort_unstable();
    serials.dedup();
    Ok(Some(format_serials(&serials)))
}

/// gallery-dl's `--range`/`--child-range` select extractor *indices*, not the
/// chapter numbers shown in the UI. `--child-filter` matches the extractor's
/// own `chapter` value, so `1-3,7` really means those chapters.
fn manga_chapter_filter(spec: &str) -> Option<String> {
    let numbers = crate::export::parse_selection_serials(spec)?;
    if numbers.is_empty() {
        return None;
    }
    let mut values = Vec::with_capacity(numbers.len() * 2);
    for number in numbers {
        values.push(format!("'{number}'"));
        values.push(format!("'{number}.0'"));
    }
    Some(format!(
        "chapter is not None and str(chapter) in ({})",
        values.join(", ")
    ))
}

/// NF-041: how many chapters this run actually crawled. An update's export
/// merges the whole book, so the explicit selection wins; a full crawl (`--all`)
/// falls back to the exported count. Never exceed what the export contains.
fn crawled_chapter_count(selection: Option<&str>, exported: u32) -> u32 {
    selected_chapter_count(selection)
        .unwrap_or(exported)
        .min(exported)
}

fn selected_chapter_count(value: Option<&str>) -> Option<u32> {
    let spec = normalize_chapters(value).ok().flatten()?;
    let mut total = 0_u32;
    for chunk in spec.split(',') {
        let count = match chunk.split_once('-') {
            Some((start, end)) => {
                let start = start.parse::<u32>().ok()?;
                let end = end.parse::<u32>().ok()?;
                end.saturating_sub(start) + 1
            }
            None => 1,
        };
        total = total.saturating_add(count);
    }
    (total > 0).then_some(total)
}

fn parse_chapter_number(value: &str, token: &str) -> AppResult<u32> {
    value.trim().parse::<u32>().map_err(|_| {
        AppError::msg(format!(
            "“{token}” is not a valid chapter. Use 12, 1-20 or 19,31,82."
        ))
    })
}

fn clean_line(raw: &str) -> String {
    let stripped = strip_ansi_escapes::strip(raw);
    let text = String::from_utf8_lossy(&stripped);
    let current = text.rsplit('\r').next().unwrap_or_default().trim();
    let filtered = current
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .collect::<String>();
    let trimmed = filtered.trim();
    trimmed.chars().take(1_600).collect()
}

fn parse_percent(line: &str) -> Option<f32> {
    use std::sync::OnceLock;

    static PERCENT: OnceLock<Regex> = OnceLock::new();
    static RATIO: OnceLock<Regex> = OnceLock::new();

    let percent_re = PERCENT
        .get_or_init(|| Regex::new(r"(?i)(\d{1,3}(?:\.\d+)?)\s*%").expect("valid percent regex"));
    if let Some(captures) = percent_re.captures(line)
        && let Some(value) = captures
            .get(1)
            .and_then(|capture| capture.as_str().parse::<f32>().ok())
    {
        return Some(value.clamp(0.0, 100.0));
    }

    let ratio_re = RATIO.get_or_init(|| {
        Regex::new(r"(?i)(\d{1,6})\s*(?:/|of)\s*(\d{1,6})").expect("valid ratio regex")
    });
    if let Some(captures) = ratio_re.captures(line) {
        let current = captures.get(1)?.as_str().parse::<f32>().ok()?;
        let total = captures.get(2)?.as_str().parse::<f32>().ok()?;
        if (2.0..=100_000.0).contains(&total) && current <= total {
            return Some((current / total * 100.0).clamp(0.0, 100.0));
        }
    }

    None
}

fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .max_depth(5)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

#[derive(Default)]
struct WorkStats {
    bytes: u64,
    chapters: u32,
    images: u32,
}

/// One walk over the isolated crawler store gives both byte progress and the
/// chapter/image file counts used when the engine suppresses its progress bars.
/// lncrawl names its folders `chapters`/`images`; gallery-dl nests images one
/// folder per chapter, so every image-bearing directory counts as a chapter.
fn work_stats(path: &Path, engine: Engine) -> WorkStats {
    let mut stats = WorkStats::default();
    let mut manga_chapters = std::collections::HashSet::<PathBuf>::new();
    for entry in WalkDir::new(path)
        .max_depth(6)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let Some(metadata) = entry.metadata().ok() else {
            continue;
        };
        stats.bytes += metadata.len();
        let file_name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if engine == Engine::Manga {
            if is_manga_image(&file_name) {
                stats.images += 1;
                if let Some(parent) = entry.path().parent() {
                    manga_chapters.insert(parent.to_path_buf());
                }
            }
            continue;
        }
        let parent = entry
            .path()
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        match parent {
            "chapters" => stats.chapters += 1,
            "images" => stats.images += 1,
            _ => {}
        }
    }
    if engine == Engine::Manga {
        stats.chapters = manga_chapters.len() as u32;
    }
    stats
}

fn is_manga_image(name: &str) -> bool {
    [".jpg", ".jpeg", ".png", ".webp", ".gif", ".avif"]
        .iter()
        .any(|extension| name.ends_with(extension))
}

/// True for UUID/hash slugs that make poor folder and archive names.
fn is_opaque_title(title: &str) -> bool {
    let compact = title
        .chars()
        .filter(|value| value.is_ascii_hexdigit())
        .count();
    let total = title.chars().count();
    total >= 24 && compact >= 24 && compact * 4 >= total * 3
}

/// `gallery-dl` writes `<root>/<category>/<series>/<chapter>/`. Return the
/// series folder name when exactly one series was downloaded.
fn gallery_dl_series_title(root: &Path) -> Option<String> {
    let mut series: Vec<PathBuf> = Vec::new();
    let categories = std::fs::read_dir(root).ok()?;
    for category in categories.filter_map(Result::ok) {
        if !category
            .file_type()
            .map(|kind| kind.is_dir())
            .unwrap_or(false)
        {
            continue;
        }
        let category_path = category.path();
        let Ok(children) = std::fs::read_dir(&category_path) else {
            continue;
        };
        for child in children.filter_map(Result::ok) {
            if !child.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let path = child.path();
            let has_images = WalkDir::new(&path)
                .max_depth(3)
                .follow_links(false)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file())
                .any(|entry| {
                    is_manga_image(&entry.file_name().to_string_lossy().to_ascii_lowercase())
                });
            if has_images {
                series.push(path);
            }
        }
    }
    match series.as_slice() {
        [only] => only
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

/// Reads the first `info.json` written by `gallery-dl --write-info-json` and
/// maps its extractor fields onto the app's metadata model. Every chapter of a
/// series repeats the series fields, so the first file is enough.
fn gallery_dl_metadata(work_dir: &Path) -> Option<NovelMetadata> {
    let root = work_dir.join("gallery-dl");
    let info = WalkDir::new(&root)
        .max_depth(6)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| {
            entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("info.json")
        })?;
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(info.path()).ok()?).ok()?;

    let title = data
        .get("manga")
        .and_then(serde_json::Value::as_str)
        .or_else(|| data.get("title").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string();

    let mut authors = json_strings(data.get("author"));
    for artist in json_strings(data.get("artist")) {
        if !authors.iter().any(|existing| existing == &artist) {
            authors.push(artist);
        }
    }

    Some(NovelMetadata {
        title,
        author: authors.first().cloned(),
        authors,
        synopsis: data
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        tags: json_strings(data.get("tags"))
            .into_iter()
            .take(24)
            .collect(),
        status: None,
        language: data
            .get("lang")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        chapter_count: None,
        source_url: String::new(),
        source: data
            .get("category")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        cover: None,
        aliases: Vec::new(),
    })
}

fn json_strings(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.as_str().map(str::to_string).or_else(|| {
                    item.get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
            })
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect(),
        Some(serde_json::Value::String(value)) => {
            let value = value.trim().to_string();
            if value.is_empty() {
                Vec::new()
            } else {
                vec![value]
            }
        }
        _ => Vec::new(),
    }
}

fn non_blank(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn scan_outputs(directory: &Path) -> Vec<PathBuf> {
    let mut files = WalkDir::new(directory)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "epub" | "cbz" | "zip" | "pdf" | "mobi" | "azw3" | "txt"
                    )
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn output_file(path: &Path, verified: Option<VerificationReport>) -> OutputFile {
    let size = std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    OutputFile {
        path: path.to_string_lossy().to_string(),
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "output".to_string()),
        size,
        kind: path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase(),
        verified,
    }
}

fn update_task(
    state: &AppState,
    job_id: &str,
    update: impl FnOnce(&mut DownloadTask),
) -> Option<DownloadTask> {
    let mut jobs = state.jobs.lock().ok()?;
    let record = jobs.get_mut(job_id)?;
    let previous = record.task.status;
    update(&mut record.task);
    if record.task.status.is_terminal() {
        record.cancel = None;
        record.pid = 0;
    }
    let status_changed = previous != record.task.status;
    let task = record.task.clone();
    drop(jobs);
    // Progress ticks update the in-memory task only; the journal is written
    // when a job actually changes state.
    if status_changed {
        state.persist_journal();
    }
    Some(task)
}

/// NF-041: estimated crawl window for a selection at a measured rate. Rounded
/// up so a 10.5-minute estimate is not shown as 10 minutes. Returns 0 when no
/// pace or no chapters are known.
pub fn expected_seconds_for(chapters: u32, chapters_per_minute: f64) -> u64 {
    if chapters == 0 || !chapters_per_minute.is_finite() || chapters_per_minute <= 0.0 {
        return 0;
    }
    (f64::from(chapters) / chapters_per_minute * 60.0).ceil() as u64
}

/// NF-041: attach the measured pace to a job that is updating a folder with a
/// previous measurement. Emits the task and returns the updated copy so callers
/// do not hand a stale task (with empty expected fields) back to the UI.
pub fn annotate_expected_pace(
    app: &AppHandle,
    state: &AppState,
    job_id: &str,
    chapters_per_minute: f64,
    expected_seconds: u64,
) -> Option<DownloadTask> {
    let task = update_task(state, job_id, |task| {
        task.expected_chapters_per_minute = Some(chapters_per_minute);
        task.expected_seconds = Some(expected_seconds);
    });
    if let Some(task) = &task {
        emit_task(app, task);
    }
    task
}

/// The matching dock line: it names the host, the measured rate and the
/// expected crawl window so a slow source is explainable while it runs.
pub fn log_expected_pace(
    app: &AppHandle,
    job_id: &str,
    host: &str,
    chapters_per_minute: f64,
    chapters: u32,
    expected_seconds: u64,
) {
    log_system(
        app,
        job_id,
        format!(
            "Measured pace · {chapters_per_minute:.1} chapters/min on {host} · {chapters} chapter(s) selected · expected crawl {}",
            format_duration(Duration::from_secs(expected_seconds))
        ),
    );
}

fn emit_task(app: &AppHandle, task: &DownloadTask) {
    let _ = app.emit(EVENT_STATUS, StatusEvent { task: task.clone() });
}

/// Move the visible pipeline phase and push a status event immediately. The
/// progress sampler will keep reporting bytes/speed for the new phase.
fn set_phase(
    state: &AppState,
    app: &AppHandle,
    job_id: &str,
    phase: TaskPhase,
) -> Option<DownloadTask> {
    let mut changed = false;
    let task = update_task(state, job_id, |task| {
        changed = task.phase != phase;
        task.phase = phase;
    })?;
    if changed {
        log_system(app, job_id, format!("Phase · {}", phase.label()));
    }
    emit_task(app, &task);
    Some(task)
}

fn log_system(app: &AppHandle, job_id: &str, line: impl Into<String>) {
    emit_log(app, job_id, LogStream::System, line.into());
}

fn log_stderr(app: &AppHandle, job_id: &str, line: impl Into<String>) {
    emit_log(app, job_id, LogStream::Stderr, line.into());
}

fn emit_log(app: &AppHandle, job_id: &str, stream: LogStream, line: String) {
    append_log_file(app, job_id, stream, &line);
    let _ = app.emit(
        EVENT_LOG,
        LogEvent {
            job_id: job_id.to_string(),
            stream,
            line,
            ts: now_ms(),
        },
    );
}

/// Serialises writers so two engine pumps cannot interleave a half line.
static LOG_FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Rotate before the file grows past 5 MiB. One old generation is enough for a
/// bug report; a diagnostics export includes only the tail of the current file.
const LOG_ROTATE_BYTES: u64 = 5 * 1024 * 1024;

fn append_log_file(app: &AppHandle, job_id: &str, stream: LogStream, line: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let path = state.log_file.as_ref();
    let Ok(_guard) = LOG_FILE_LOCK.lock() else {
        return;
    };
    if let Ok(metadata) = std::fs::metadata(path)
        && metadata.len() >= LOG_ROTATE_BYTES
    {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let stream = match stream {
        LogStream::Stdout => "stdout",
        LogStream::Stderr => "stderr",
        LogStream::System => "system",
    };
    let line = format!("{} {stream:<6} {job_id} {line}\n", iso_timestamp());
    let _ = std::io::Write::write_all(&mut file, line.as_bytes());
}

/// Last `max_lines` lines of the current log file, oldest first.
pub fn log_tail(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut lines = text.lines().collect::<Vec<_>>();
    if lines.len() > max_lines {
        lines = lines.split_off(lines.len() - max_lines);
    }
    lines.into_iter().map(str::to_string).collect()
}

/// UTC timestamp for log rows. Implemented here so the release binary keeps
/// one fewer dependency; the log is only read by humans and diagnostics.
fn iso_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = secs.div_euclid(86_400);
    let time = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

/// Collected artifact candidates plus the in-progress Rich line wrap.
#[derive(Default)]
struct ArtifactCapture {
    files: Vec<PathBuf>,
    pending: Vec<String>,
    active: bool,
}

/// lncrawl stores artifacts under its own data directory and prints the final
/// path as `epub (size): /path`. When stdout is a pipe, Rich wraps that path at
/// its detected width — sometimes mid-word — so the path has to be reassembled
/// from the following lines. Capture it here so the job sandbox receives the
/// real output (the crawler offers no output-directory flag).
fn capture_artifact_line(line: &str, sink: &Mutex<ArtifactCapture>) {
    let Ok(mut sink) = sink.lock() else {
        return;
    };

    if sink.active {
        sink.pending.push(line.trim().to_string());
        if sink.pending.len() > 12 {
            sink.pending.clear();
            sink.active = false;
            return;
        }
        if let Some(path) = resolve_wrapped_path(&sink.pending) {
            if !sink.files.contains(&path) {
                sink.files.push(path);
            }
            sink.pending.clear();
            sink.active = false;
        }
        return;
    }

    let Some(candidate) = artifact_summary_path(line) else {
        return;
    };
    let path =
        PathBuf::from(candidate.trim_matches(|character| character == '"' || character == '\''));
    if path.is_absolute() && path.is_file() && has_output_extension(&path) {
        if !sink.files.contains(&path) {
            sink.files.push(path);
        }
        return;
    }

    sink.active = true;
    sink.pending.clear();
    if !candidate.trim().is_empty() {
        sink.pending.push(candidate.trim().to_string());
    }
}

/// Returns the text after `epub (27.0 KB):` when the line is an artifact summary.
fn artifact_summary_path(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let (prefix, rest) = trimmed.split_once(':')?;
    if !prefix.contains('(') || !prefix.ends_with(')') {
        return None;
    }
    let format = prefix.split_whitespace().next()?.to_ascii_lowercase();
    if !matches!(
        format.as_str(),
        "epub" | "cbz" | "zip" | "pdf" | "mobi" | "azw3" | "txt"
    ) {
        return None;
    }
    Some(rest)
}

/// Rich may break a path at any character. Try joining the wrapped segments
/// with and without the spaces that the wrap may have consumed.
fn resolve_wrapped_path(segments: &[String]) -> Option<PathBuf> {
    if segments.is_empty() {
        return None;
    }
    let boundaries = segments.len().saturating_sub(1);
    if boundaries > 10 {
        return None;
    }
    for mask in 0..(1_u32 << boundaries) {
        let mut joined = segments[0].clone();
        for (index, segment) in segments.iter().enumerate().skip(1) {
            if mask & (1 << (index - 1)) != 0 {
                joined.push(' ');
            }
            joined.push_str(segment);
        }
        let path = PathBuf::from(joined.trim());
        if path.is_absolute() && path.is_file() && has_output_extension(&path) {
            return Some(path);
        }
    }
    None
}

fn import_lncrawl_artifacts(
    app: &AppHandle,
    job_id: &str,
    captured: &Mutex<ArtifactCapture>,
    destination: &Path,
    since: SystemTime,
    work_dir: Option<&Path>,
) {
    let mut sources = captured
        .lock()
        .map(|guard| guard.files.clone())
        .unwrap_or_default();

    if sources.is_empty() {
        sources = recent_lncrawl_artifacts(since, work_dir);
        if !sources.is_empty() {
            log_system(
                app,
                job_id,
                format!(
                    "Artifact path not printed — recovered {} recent artifact(s) from the lncrawl store",
                    sources.len()
                ),
            );
        }
    }

    for source in sources {
        if !source.is_file() || source.starts_with(destination) {
            continue;
        }
        let Some(name) = source.file_name().map(|name| name.to_owned()) else {
            continue;
        };
        let target = unique_target(destination, &name);
        let moved = std::fs::rename(&source, &target)
            .or_else(|_| std::fs::copy(&source, &target).map(|_| ()));

        match moved {
            Ok(()) => log_system(
                app,
                job_id,
                format!(
                    "Imported lncrawl artifact {} into the job sandbox",
                    target
                        .file_name()
                        .map(|value| value.to_string_lossy().to_string())
                        .unwrap_or_default()
                ),
            ),
            Err(error) => log_stderr(
                app,
                job_id,
                format!("Could not import artifact {}: {error}", source.display()),
            ),
        }
    }
}

fn recent_lncrawl_artifacts(since: SystemTime, work_dir: Option<&Path>) -> Vec<PathBuf> {
    let cutoff = since.checked_sub(Duration::from_secs(15)).unwrap_or(since);
    let mut roots = Vec::new();
    if let Some(directory) = work_dir {
        // An isolated work dir belongs to exactly this job, so there is no
        // chance of importing another job's artifact.
        roots.push(directory.to_path_buf());
    } else {
        if let Some(configured) = std::env::var_os("LNCRAWL_DATA_PATH") {
            roots.push(PathBuf::from(configured));
        }
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".lncrawl"));
            roots.push(home.join(".config/LNCrawl"));
        }
    }

    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    for root in roots {
        let novels = root.join("novels");
        if !novels.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&novels)
            .max_depth(4)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            if !has_output_extension(path) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            if modified >= cutoff {
                found.push((modified, path.to_path_buf()));
            }
        }
    }

    // Newest first so a job that failed to print its artifact path still gets
    // the freshest candidate before older leftovers.
    found.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    found.dedup_by(|left, right| left.1 == right.1);
    found.into_iter().map(|(_, path)| path).collect()
}

fn has_output_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "epub" | "cbz" | "zip" | "pdf" | "mobi" | "azw3" | "txt"
            )
        })
        .unwrap_or(false)
}

fn unique_target(directory: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let candidate = directory.join(name);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let extension = path
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();

    for index in 2..=999 {
        let next = directory.join(format!("{stem}-{index}{extension}"));
        if !next.exists() {
            return next;
        }
    }
    directory.join(format!("{stem}-copy{extension}"))
}

fn prune_jobs(jobs: &mut HashMap<String, JobRecord>) {
    let mut finished = jobs
        .iter()
        .filter(|(_, record)| record.task.status.is_terminal())
        .map(|(id, record)| (id.clone(), record.task.created_at))
        .collect::<Vec<_>>();

    if finished.len() >= 40 {
        finished.sort_by_key(|(_, created_at)| *created_at);
        for (id, _) in finished.into_iter().take(20) {
            jobs.remove(&id);
        }
    }

    if jobs.len() > 80 {
        let mut finished = jobs
            .iter()
            .filter(|(_, record)| record.task.status.is_terminal())
            .map(|(id, record)| (id.clone(), record.task.created_at))
            .collect::<Vec<_>>();
        finished.sort_by_key(|(_, created_at)| *created_at);
        let excess = jobs.len().saturating_sub(60);
        for (id, _) in finished.into_iter().take(excess) {
            jobs.remove(&id);
        }
    }
}

#[cfg(test)]
mod artifact_tests {
    use super::*;

    #[test]
    fn recognises_artifact_summary_lines() {
        assert_eq!(
            artifact_summary_path("epub (27.0 KB): /tmp/book.epub"),
            Some(" /tmp/book.epub")
        );
        assert_eq!(
            artifact_summary_path("pdf (1.2 MB): /tmp/book.pdf"),
            Some(" /tmp/book.pdf")
        );
        assert_eq!(artifact_summary_path("[14:47:02] INFO Created"), None);
        assert_eq!(artifact_summary_path("INFO epub (27.0 KB) created"), None);
    }

    #[test]
    fn reassembles_mid_word_wraps() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-wrap-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let file = root.join("The Tell Tale Heart (1843).epub");
        std::fs::write(&file, b"fixture").expect("write fixture");

        let text = file.to_string_lossy().to_string();
        let split = text.len() / 2;
        let first = text[..split].to_string();
        let second = text[split..].to_string();
        let resolved = resolve_wrapped_path(&[first, second]);
        assert_eq!(resolved.as_deref(), Some(file.as_path()));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn normalises_chapter_selections() {
        assert_eq!(normalize_chapters(None).unwrap(), None);
        assert_eq!(normalize_chapters(Some("all")).unwrap(), None);
        assert_eq!(
            normalize_chapters(Some("19,31,82,38,99"))
                .unwrap()
                .as_deref(),
            Some("19,31,38,82,99")
        );
        assert_eq!(
            normalize_chapters(Some("20-1")).unwrap().as_deref(),
            Some("1-20")
        );
        assert_eq!(
            normalize_chapters(Some("1-3,4,5,9")).unwrap().as_deref(),
            Some("1-5,9")
        );
        assert!(normalize_chapters(Some("one")).is_err());
    }

    #[test]
    fn folder_busy_guard_ignores_terminal_jobs() {
        let folder = std::env::temp_dir().join(format!(
            "novelforge-busy-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&folder).expect("create temp dir");
        let canonical = folder.canonicalize().expect("canonical folder");
        let state = AppState::new(
            Settings::default(),
            PathBuf::from("/tmp/novelforge-test-config.json"),
        );
        let task = DownloadTask {
            id: "busy-job".to_string(),
            title: "Busy".to_string(),
            query: "https://example.com/book".to_string(),
            engine: Engine::Webnovel,
            status: TaskStatus::Running,
            phase: TaskPhase::Resolving,
            created_at: 0,
            destination: canonical.to_string_lossy().to_string(),
            progress: 0.0,
            bytes: 0,
            speed_bps: 0.0,
            eta_seconds: None,
            elapsed_seconds: 0,
            idle_seconds: 0,
            expected_chapters_per_minute: None,
            expected_seconds: None,
            message: None,
            files: Vec::new(),
        };
        state.jobs.lock().expect("jobs").insert(
            "busy-job".to_string(),
            JobRecord {
                task: task.clone(),
                pid: 0,
                cancel: None,
            },
        );

        assert!(folder_is_busy(&state, &canonical));
        state
            .jobs
            .lock()
            .expect("jobs")
            .get_mut("busy-job")
            .expect("job")
            .task
            .status = TaskStatus::Completed;
        assert!(!folder_is_busy(&state, &canonical));

        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn book_args_carry_verified_metadata_and_libgen_source() {
        let settings = Settings::default();
        let destination = std::env::temp_dir();
        let request = DownloadRequest {
            engine: Engine::Books,
            query: "Fourth Wing".to_string(),
            title: Some("Fourth Wing".to_string()),
            options: DownloadOptions {
                book_source: Some("libgen".to_string()),
                book_title: Some("Fourth Wing".to_string()),
                book_author: Some("Rebecca Yarros".to_string()),
                book_isbn: Some("9781649374042".to_string()),
                ..DownloadOptions::default()
            },
        };

        let args = build_args(&request, &settings, &destination, None).expect("build book args");
        let pairs = args
            .windows(2)
            .map(|window| (window[0].as_str(), window[1].as_str()))
            .collect::<Vec<_>>();
        assert!(pairs.contains(&("--source", "libgen")));
        assert!(pairs.contains(&("--title", "Fourth Wing")));
        assert!(pairs.contains(&("--author", "Rebecca Yarros")));
        assert!(pairs.contains(&("--isbn", "9781649374042")));
        assert!(pairs.contains(&("--format", "epub")));
        assert!(args.iter().any(|arg| arg == "--headless"));
        assert_eq!(args.last().map(String::as_str), Some("Fourth Wing"));
    }

    #[test]
    fn book_source_accepts_public_domain_sources() {
        let settings = Settings::default();
        let destination = std::env::temp_dir();
        for source in ["gutenberg", "standardebooks"] {
            let request = DownloadRequest {
                engine: Engine::Books,
                query: "Pride and Prejudice".to_string(),
                title: None,
                options: DownloadOptions {
                    book_source: Some(source.to_string()),
                    ..DownloadOptions::default()
                },
            };
            let args = build_args(&request, &settings, &destination, None)
                .unwrap_or_else(|error| panic!("{source} should be accepted: {error:?}"));
            let pairs = args
                .windows(2)
                .map(|window| (window[0].as_str(), window[1].as_str()))
                .collect::<Vec<_>>();
            assert!(pairs.contains(&("--source", source)));
        }
    }

    #[test]
    fn book_source_rejects_unknown_sources() {
        let settings = Settings::default();
        let destination = std::env::temp_dir();
        let request = DownloadRequest {
            engine: Engine::Books,
            query: "Dune".to_string(),
            title: None,
            options: DownloadOptions {
                book_source: Some("piratebay".to_string()),
                ..DownloadOptions::default()
            },
        };
        assert!(build_args(&request, &settings, &destination, None).is_err());
    }

    #[test]
    fn book_validation_rejects_malformed_isbn() {
        let request = DownloadRequest {
            engine: Engine::Books,
            query: "Dune".to_string(),
            title: None,
            options: DownloadOptions {
                book_isbn: Some("123".to_string()),
                ..DownloadOptions::default()
            },
        };
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn series_layout_uses_author_series_and_padded_position() {
        let root = Path::new("/library");
        let options = DownloadOptions {
            book_author: Some("Rebecca Yarros".to_string()),
            book_series: Some("The Empyrean".to_string()),
            book_series_position: Some(1.0),
            ..DownloadOptions::default()
        };
        assert_eq!(
            book_series_destination(root, &options, "Fourth Wing"),
            Some(
                root.join("Rebecca Yarros")
                    .join("The Empyrean")
                    .join("01 - Fourth Wing")
            )
        );

        let novella = DownloadOptions {
            book_series_position: Some(3.5),
            ..options.clone()
        };
        assert_eq!(
            series_book_stem(&novella, "Threshing Day"),
            "03.5 - Threshing Day"
        );
    }

    #[test]
    fn series_layout_falls_back_without_a_position_and_stays_contained() {
        let root = Path::new("/library");
        assert_eq!(
            book_series_destination(root, &DownloadOptions::default(), "Dune"),
            None
        );

        let unpositioned = DownloadOptions {
            book_author: Some("J.R.R. Tolkien".to_string()),
            book_series: Some("The Lord of the Rings".to_string()),
            ..DownloadOptions::default()
        };
        assert_eq!(
            book_series_destination(root, &unpositioned, "The Hobbit"),
            Some(
                root.join("J.R.R. Tolkien")
                    .join("The Lord of the Rings")
                    .join("The Hobbit")
            )
        );

        let hostile = DownloadOptions {
            book_author: Some("../Escape".to_string()),
            book_series: Some("A/B".to_string()),
            book_series_position: Some(2.0),
            ..DownloadOptions::default()
        };
        let path =
            book_series_destination(root, &hostile, "../../Evil: Book").expect("series path");
        assert!(path.starts_with(root));
        assert!(!path.to_string_lossy().contains(".."));
        assert_eq!(path, root.join("Escape").join("A B").join("02 - Evil Book"));
    }

    fn chapter_index(numbers: &[u32]) -> NovelChapterIndex {
        NovelChapterIndex {
            source_url: "https://novelphoenix.net/omniscient-readers-viewpoint.html".to_string(),
            source: "novelphoenix.net".to_string(),
            title: Some("Fixture".to_string()),
            total: numbers.len() as u32,
            complete: true,
            detail: Some("fixture".to_string()),
            chapters: numbers
                .iter()
                .map(|number| NovelChapter {
                    number: *number,
                    title: format!("Chapter {number}"),
                    url: format!("https://example.com/chapter-{number}.html"),
                    volume: None,
                    downloaded: false,
                    file: None,
                })
                .collect(),
            cached: false,
            cache_age_seconds: None,
            transport: None,
            title_source: None,
            title_lookup_attempted: false,
            title_recovery_attempted: false,
        }
    }

    #[test]
    fn webnovel_selection_is_translated_from_visible_numbers_to_serials() {
        let settings = Settings::default();
        let destination = std::env::temp_dir();
        let request = DownloadRequest {
            engine: Engine::Webnovel,
            query: "https://novelphoenix.net/omniscient-readers-viewpoint.html".to_string(),
            title: None,
            options: DownloadOptions {
                chapters: Some("2-3".to_string()),
                ..DownloadOptions::default()
            },
        };
        // ORV's catalogue starts at visible Chapter 0, so visible 2 and 3 are
        // lncrawl serials 3 and 4.
        let index = chapter_index(&[0, 1, 2, 3, 4]);
        let args = build_args(&request, &settings, &destination, Some(&index))
            .expect("build webnovel args");
        let spec = args
            .iter()
            .position(|arg| arg == "--chapters")
            .and_then(|position| args.get(position + 1))
            .map(String::as_str);
        assert_eq!(spec, Some("3-4"));

        // A 1-based source maps to the same numbers.
        let index = chapter_index(&[1, 2, 3, 4]);
        let args = build_args(&request, &settings, &destination, Some(&index))
            .expect("build webnovel args");
        let spec = args
            .iter()
            .position(|arg| arg == "--chapters")
            .and_then(|position| args.get(position + 1))
            .map(String::as_str);
        assert_eq!(spec, Some("2-3"));
    }

    #[test]
    fn webnovel_selection_fails_closed_without_a_matching_index_row() {
        let settings = Settings::default();
        let destination = std::env::temp_dir();
        let request = DownloadRequest {
            engine: Engine::Webnovel,
            query: "https://novelphoenix.net/omniscient-readers-viewpoint.html".to_string(),
            title: None,
            options: DownloadOptions {
                chapters: Some("111".to_string()),
                ..DownloadOptions::default()
            },
        };
        // ORV's table of contents has no visible Chapter 111.
        let index = chapter_index(&[109, 110, 112, 113]);
        let error = build_args(&request, &settings, &destination, Some(&index))
            .expect_err("a missing chapter must not be mapped to a neighbour");
        assert!(error.to_string().contains("Chapter 111"));

        // Without the list, the visible number cannot be trusted as a serial.
        let error = build_args(&request, &settings, &destination, None)
            .expect_err("a partial run without an index must fail closed");
        assert!(error.to_string().contains("chapter list"));
    }

    #[test]
    fn webnovel_selection_refuses_an_incomplete_or_ambiguous_index() {
        let settings = Settings::default();
        let destination = std::env::temp_dir();
        let request = DownloadRequest {
            engine: Engine::Webnovel,
            query: "https://novelphoenix.net/omniscient-readers-viewpoint.html".to_string(),
            title: None,
            options: DownloadOptions {
                chapters: Some("1".to_string()),
                ..DownloadOptions::default()
            },
        };

        let incomplete = NovelChapterIndex {
            complete: false,
            detail: Some("only 7 of 9 TOC pages reached".to_string()),
            ..chapter_index(&[0, 1, 2])
        };
        let error = build_args(&request, &settings, &destination, Some(&incomplete))
            .expect_err("an unproven index must not be used");
        assert!(error.to_string().contains("incomplete"));

        let repeated = chapter_index(&[0, 1, 1, 2]);
        let error = build_args(&request, &settings, &destination, Some(&repeated))
            .expect_err("a repeated number cannot be mapped");
        assert!(error.to_string().contains("more than once"));
    }

    #[test]
    fn webnovel_throttle_maps_to_lncrawl_rate_limit() {
        let settings = Settings::default();
        let destination = std::env::temp_dir();
        let request = DownloadRequest {
            engine: Engine::Webnovel,
            query: "https://novelfire.net/book/reverend-insanity".to_string(),
            title: None,
            options: DownloadOptions {
                chapters: Some("1-3".to_string()),
                // Must be ignored for webnovels: the CLI has no jobs flag.
                jobs: Some(16),
                throttle: Some(true),
                book_source: None,
                book_title: None,
                book_author: None,
                book_isbn: None,
                book_series: None,
                book_series_position: None,
                duplicate: None,
                target_folder: None,
            },
        };

        let index = chapter_index(&[1, 2, 3]);
        let args = build_args(&request, &settings, &destination, Some(&index))
            .expect("build webnovel args");
        let rate = args
            .iter()
            .position(|arg| arg == "--rate-limit")
            .expect("rate limit flag");
        assert_eq!(args.get(rate + 1).map(String::as_str), Some("2"));
        assert!(!args.iter().any(|arg| arg == "-j"));
        assert!(!args.iter().any(|arg| arg == "16"));
        assert!(webnovel_transport_summary(&request.options, &request.query).contains("2/sec"));
        assert!(webnovel_transport_summary(&request.options, &request.query).contains("NovelFire"));
    }

    #[test]
    fn webnovel_throttle_off_keeps_source_default_pacing() {
        let settings = Settings::default();
        let destination = std::env::temp_dir();
        let request = DownloadRequest {
            engine: Engine::Webnovel,
            query: "https://novelfire.net/book/reverend-insanity".to_string(),
            title: None,
            options: DownloadOptions {
                chapters: None,
                jobs: Some(8),
                throttle: Some(false),
                book_source: None,
                book_title: None,
                book_author: None,
                book_isbn: None,
                book_series: None,
                book_series_position: None,
                duplicate: None,
                target_folder: None,
            },
        };

        let args =
            build_args(&request, &settings, &destination, None).expect("build webnovel args");
        assert!(!args.iter().any(|arg| arg == "--rate-limit"));
        assert!(!args.iter().any(|arg| arg == "-j"));
        assert!(
            webnovel_transport_summary(&request.options, &request.query).contains("source default")
        );
    }

    #[test]
    fn reassembles_space_wraps() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-wrap-space-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let file = root.join("A Book With Spaces.epub");
        std::fs::write(&file, b"fixture").expect("write fixture");

        let text = file.to_string_lossy().to_string();
        let index = text.find(" With").expect("space marker");
        let resolved =
            resolve_wrapped_path(&[text[..index].to_string(), text[index + 1..].to_string()]);
        assert_eq!(resolved.as_deref(), Some(file.as_path()));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_lncrawl_progress_stages() {
        let chapters =
            parse_progress_line("Chapters:  42%|████      | 42/100 [00:12<00:16,  3.5 c/s]")
                .expect("chapter bar");
        assert_eq!(chapters.phase, TaskPhase::DownloadingChapters);
        assert_eq!(chapters.percent, Some(42.0));
        assert_eq!(chapters.counts, Some((42, 100)));

        let images = parse_progress_line("Images:  10%|█         | 2/20 [00:01<00:08,  2.1 img/s]")
            .expect("image bar");
        assert_eq!(images.phase, TaskPhase::DownloadingImages);
        assert_eq!(images.counts, Some((2, 20)));

        let packing = parse_progress_line("Generating epub...").expect("packing status");
        assert_eq!(packing.phase, TaskPhase::Packing);
        assert_eq!(packing.percent, None);

        // Chapter prose must never be mistaken for a progress bar.
        assert!(parse_progress_line("Chapter 7: 100% of 12/40 words").is_none());
        assert!(parse_progress_line("INFO Chapters downloaded").is_none());
    }

    #[test]
    fn eta_uses_recent_percent_movement_only() {
        let now = Instant::now();
        let mut steady = VecDeque::new();
        steady.push_back((now - Duration::from_secs(10), 20.0));
        steady.push_back((now, 40.0));
        let eta = eta_from_samples(&steady, 40.0).expect("steady eta");
        assert!((28..=32).contains(&eta), "unexpected eta {eta}");

        let mut flat = VecDeque::new();
        flat.push_back((now - Duration::from_secs(10), 20.0));
        flat.push_back((now, 20.0));
        assert_eq!(eta_from_samples(&flat, 20.0), None);

        let mut backwards = VecDeque::new();
        backwards.push_back((now - Duration::from_secs(10), 40.0));
        backwards.push_back((now, 30.0));
        assert_eq!(eta_from_samples(&backwards, 30.0), None);

        // A single sample is not enough evidence for an estimate.
        let mut single = VecDeque::new();
        single.push_back((now, 50.0));
        assert_eq!(eta_from_samples(&single, 50.0), None);
    }

    #[test]
    fn phase_codes_round_trip() {
        for phase in [
            TaskPhase::Resolving,
            TaskPhase::FetchingDetails,
            TaskPhase::DownloadingChapters,
            TaskPhase::DownloadingImages,
            TaskPhase::Packing,
            TaskPhase::Importing,
            TaskPhase::FetchingIndex,
            TaskPhase::Auditing,
            TaskPhase::Retrying,
            TaskPhase::Exporting,
            TaskPhase::Verifying,
            TaskPhase::Completed,
        ] {
            assert_eq!(TaskPhase::from_code(phase.code()), phase);
        }
    }

    #[test]
    fn crawled_chapter_count_uses_the_run_selection_not_the_merged_export() {
        // Update of chapters 20 and 21 in a 53-chapter book.
        assert_eq!(crawled_chapter_count(Some("20,21"), 53), 2);
        assert_eq!(crawled_chapter_count(Some("1-2"), 2), 2);
        assert_eq!(crawled_chapter_count(None, 53), 53);
        // A partial export can never claim more than it contains.
        assert_eq!(crawled_chapter_count(Some("1-5"), 3), 3);
    }

    #[test]
    fn selected_chapter_count_matches_the_lncrawl_spec() {
        assert_eq!(selected_chapter_count(Some("1-5")), Some(5));
        assert_eq!(selected_chapter_count(Some("1-3,5")), Some(4));
        assert_eq!(selected_chapter_count(Some("20-1")), Some(20));
        assert_eq!(selected_chapter_count(None), None);
        assert_eq!(selected_chapter_count(Some("all")), None);
    }

    #[test]
    fn parses_advertised_chapter_totals() {
        assert_eq!(parse_advertised_total("2 volumes, 113 chapters"), Some(113));
        assert_eq!(
            parse_advertised_total("23 volumes, 2265 chapters"),
            Some(2265)
        );
        assert_eq!(parse_advertised_total("Chapter 7 of 12"), None);
        assert_eq!(parse_advertised_total("113 chapters"), None);
    }

    #[test]
    fn work_stats_counts_chapter_and_image_files() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-stats-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let chapters = root.join("novels/id/chapters");
        let images = root.join("novels/id/images");
        std::fs::create_dir_all(&chapters).expect("create chapters");
        std::fs::create_dir_all(&images).expect("create images");
        std::fs::write(chapters.join("000001.zst"), b"chapter-one").expect("chapter");
        std::fs::write(chapters.join("000002.zst"), b"chapter-two").expect("chapter");
        std::fs::write(images.join("cover.jpg"), b"image").expect("image");

        let stats = work_stats(&root.join("novels"), Engine::Webnovel);
        assert_eq!(stats.chapters, 2);
        assert_eq!(stats.images, 1);
        assert_eq!(stats.bytes, 11 + 11 + 5);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn builds_manga_chapter_filter_from_numbers() {
        assert_eq!(manga_chapter_filter("all"), None);
        assert_eq!(
            manga_chapter_filter("1-2"),
            Some("chapter is not None and str(chapter) in ('1', '1.0', '2', '2.0')".to_string())
        );
        assert!(manga_chapter_filter("3,7").is_some_and(|value| value.contains("'7'")));
    }

    #[test]
    fn recognises_opaque_manga_titles() {
        assert!(is_opaque_title("831b12b8-2d0e-4397-8719-1efee4c32f40"));
        assert!(!is_opaque_title("Oshimai"));
        assert!(!is_opaque_title("Solo Leveling"));
    }

    #[test]
    fn reads_gallery_dl_info_json() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-gdl-meta-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let chapter = root.join("gallery-dl/mangadex/Example/v01 c001");
        std::fs::create_dir_all(&chapter).expect("create chapter");
        std::fs::write(
            chapter.join("info.json"),
            br#"{
                "category": "mangadex",
                "manga": "Example Series",
                "title": "Chapter One",
                "author": ["A. Writer"],
                "artist": ["B. Artist"],
                "lang": "en",
                "tags": ["Action", "Drama"],
                "description": "A real description."
            }"#,
        )
        .expect("write info");

        let metadata = gallery_dl_metadata(&root).expect("metadata");
        assert_eq!(metadata.title, "Example Series");
        assert_eq!(metadata.author.as_deref(), Some("A. Writer"));
        assert_eq!(metadata.authors, vec!["A. Writer", "B. Artist"]);
        assert_eq!(metadata.tags, vec!["Action", "Drama"]);
        assert_eq!(metadata.language.as_deref(), Some("en"));
        assert_eq!(metadata.synopsis.as_deref(), Some("A real description."));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn work_stats_counts_manga_chapters_and_pages() {
        let root = std::env::temp_dir().join(format!(
            "novelforge-manga-stats-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let first = root.join("gallery-dl/mangadex/Example/v01 c001");
        let second = root.join("gallery-dl/mangadex/Example/v01 c002");
        std::fs::create_dir_all(&first).expect("create chapter");
        std::fs::create_dir_all(&second).expect("create chapter");
        std::fs::write(first.join("001.png"), b"page-one").expect("page");
        std::fs::write(first.join("002.png"), b"page-two").expect("page");
        std::fs::write(second.join("001.jpg"), b"page-three").expect("page");
        std::fs::write(second.join("info.json"), b"{}").expect("metadata");

        let stats = work_stats(&root.join("gallery-dl"), Engine::Manga);
        assert_eq!(stats.chapters, 2);
        assert_eq!(stats.images, 3);
        assert_eq!(stats.bytes, 8 + 8 + 10 + 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn error_tail_prefers_the_headline_failure_block() {
        let mut tail = ErrorTail::default();
        tail.push("[15:23:25] INFO     Sources are up to date");
        tail.push("Exhausted: no detection layer for");
        tail.push("https://freewebnovel.com/novel/reverend-insanity: gave up after 5 attempts;");
        tail.push("transport failure (Timeout) [retry -> direct after 1.0s]");

        let summary = tail.summary().expect("summary");
        assert!(summary.starts_with("Exhausted: no detection layer for"));
        assert!(summary.contains("gave up after 5 attempts"));
        assert!(!summary.contains("Sources are up to date"));
    }

    #[test]
    fn error_tail_falls_back_to_the_last_line() {
        let mut tail = ErrorTail::default();
        tail.push("Working tree ready");
        tail.push("no artifact was produced");

        assert_eq!(tail.summary().as_deref(), Some("no artifact was produced"));
        assert!(ErrorTail::default().summary().is_none());
    }

    #[test]
    fn mangadex_licensing_names_one_external_chapter() {
        let mut tail = ErrorTail::default();
        tail.push(
            "[mangadex][error] Chapter 7 is not available on MangaDex and can instead be read \
on the official publisher's website at https://tapas.io/episode/1123711.",
        );

        let note = tail.licensing_note().expect("licensing note");
        assert!(note.contains("chapter 7 is"), "{note}");
        assert!(note.contains("read at tapas.io"), "{note}");
        assert!(note.contains("publisher page is never scraped"), "{note}");
        // Full URLs stay in the logged detail rather than the job row.
        let detail = tail.licensing_detail().expect("licensing detail");
        assert!(
            detail.contains("https://tapas.io/episode/1123711"),
            "{detail}"
        );
        // The raw error line still reaches the tail summary for non-licensing use.
        assert!(tail.summary().is_some());
    }

    #[test]
    fn mangadex_licensing_counts_far_past_the_line_buffer() {
        let mut tail = ErrorTail::default();
        for chapter in 1..=40 {
            tail.push(&format!(
                "[mangadex][error] Chapter {chapter} is not available on MangaDex and can \
instead be read on the official publisher's website at https://tapas.io/episode/{chapter}."
            ));
        }

        let note = tail.licensing_note().expect("licensing note");
        assert!(note.contains("40 chapters are"), "{note}");
        let detail = tail.licensing_detail().expect("licensing detail");
        assert!(detail.contains("https://tapas.io/episode/1"), "{detail}");
        // Only the ring buffer is retained, but the count is complete.
        assert_eq!(tail.lines.len(), ERROR_TAIL_LINES);
    }

    #[test]
    fn mangadex_licensing_counts_one_chapter_across_editions() {
        // Solo Leveling prints one external-only line per language edition for
        // the same chapter; that is one unavailable chapter, not four.
        let mut tail = ErrorTail::default();
        for host in [
            "https://br.comikey.com/read/solo-leveling-webtoon/e4VGdG/episode-1/",
            "https://www.webnovel.com/comic/15227640605485101/45196190333068497",
            "https://www.tappytoon.com/en/chapters/623723039",
            "https://www.pocketcomics.com/comic/320/chapter/2/product",
        ] {
            tail.push(&format!(
                "[mangadex][error] Chapter 1 is not available on MangaDex and can instead be \
read on the official publisher's website at {host}."
            ));
        }

        let note = tail.licensing_note().expect("licensing note");
        assert!(note.contains("chapter 1 is"), "{note}");
        assert!(
            !note.contains("4 chapters"),
            "deduped chapter count leaked into note: {note}"
        );
        assert!(note.contains("and 3 other publisher(s)"), "{note}");
        assert_eq!(tail.lines.len(), 4);
    }

    #[test]
    fn unrelated_errors_do_not_claim_a_licensing_gap() {
        let mut tail = ErrorTail::default();
        tail.push("[mangadex][error] HTTP 503 while fetching chapter 4");
        assert!(tail.licensing_note().is_none());
        assert!(tail.summary().is_some());
    }
}

/// NF-035: a stalled crawl must fail within a bound, and a zero-output
/// transient gets exactly one retry while validation failures never retry.
#[cfg(test)]
mod stall_tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "novelforge-{label}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        root
    }

    #[test]
    fn stall_bound_tightens_once_chapters_flow() {
        assert_eq!(stall_limit_seconds(TaskPhase::FetchingDetails), 900);
        assert_eq!(stall_limit_seconds(TaskPhase::FetchingIndex), 900);
        assert_eq!(stall_limit_seconds(TaskPhase::DownloadingChapters), 300);
        assert_eq!(stall_limit_seconds(TaskPhase::DownloadingImages), 300);
    }

    #[test]
    fn resume_swaps_all_for_resume_and_keeps_everything_else() {
        let args = vec![
            "-ll".to_string(),
            "crawl".to_string(),
            "https://example.com/book".to_string(),
            "-f".to_string(),
            "epub".to_string(),
            "--all".to_string(),
            "--noin".to_string(),
        ];
        let resumed = resume_args(&args);
        assert!(!resumed.iter().any(|arg| arg == "--all"));
        assert_eq!(resumed.iter().filter(|arg| *arg == "--resume").count(), 1);
        assert_eq!(resumed.len(), args.len());
        assert_eq!(resumed[2], "https://example.com/book");
    }

    #[test]
    fn lncrawl_env_pins_the_database_to_the_work_dir() {
        let work = Path::new("/library/.novelforge-work/job-1");
        let mut command = Command::new("lncrawl");
        lncrawl_data_env(&mut command, work);
        let env: HashMap<_, _> = command
            .as_std()
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(|value| value.to_owned())))
            .collect();
        assert_eq!(
            env.get(std::ffi::OsStr::new("LNCRAWL_DATA_PATH"))
                .and_then(|value| value.as_deref()),
            Some(work.as_os_str())
        );
        // sqlite3 gets an absolute path, never one derived from a canonicalised
        // `\\?\` root, which was the "unable to open database file" failure.
        let url = env
            .get(std::ffi::OsStr::new("DATABASE_URL"))
            .cloned()
            .flatten()
            .expect("database url");
        assert_eq!(
            url.to_string_lossy(),
            "sqlite:////library/.novelforge-work/job-1/sqlite.db"
        );
    }

    #[tokio::test]
    async fn watchdog_fires_on_a_silent_crawl() {
        let progress = Arc::new(JobProgress::new(TaskPhase::FetchingDetails));
        progress
            .last_activity_ms
            .store(now_ms() - 901_000, Ordering::Relaxed);
        let (idle, limit) = tokio::time::timeout(
            Duration::from_secs(2),
            stall_watch_with(Arc::clone(&progress), || false, Duration::from_millis(5)),
        )
        .await
        .expect("watchdog must fire before the timeout");
        assert!(idle >= 900, "idle {idle}");
        assert_eq!(limit, 900);
    }

    #[tokio::test]
    async fn watchdog_does_not_count_paused_time_as_silence() {
        let progress = Arc::new(JobProgress::new(TaskPhase::FetchingDetails));
        // 899 s of real silence plus any pause stays under the 900 s bound.
        progress
            .last_activity_ms
            .store(now_ms() - 899_000, Ordering::Relaxed);
        let checks = Arc::new(AtomicU32::new(0));
        let gate = Arc::clone(&checks);
        let paused_for = move || {
            let seen = gate.fetch_add(1, Ordering::Relaxed);
            // "Paused" for the first ~200 ms of 5 ms polls.
            seen < 40
        };
        let watched = tokio::time::timeout(
            Duration::from_millis(300),
            stall_watch_with(Arc::clone(&progress), paused_for, Duration::from_millis(5)),
        )
        .await;
        assert!(
            watched.is_err(),
            "the watchdog fired while the job was paused: {watched:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_hung_child_is_killed_and_reported_as_stalled() {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("sleep 30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        let mut child = command.spawn().expect("spawn sleeper");
        let pid = child.id().unwrap_or(0);
        let progress = Arc::new(JobProgress::new(TaskPhase::FetchingDetails));
        progress
            .last_activity_ms
            .store(now_ms() - 901_000, Ordering::Relaxed);
        // Keep the sender alive: dropping it would look like a cancellation.
        let (_cancel, mut cancel_rx) = oneshot::channel();
        let exit = tokio::time::timeout(
            Duration::from_secs(5),
            supervise(
                &mut child,
                pid,
                &mut cancel_rx,
                &progress,
                || false,
                Duration::from_millis(5),
            ),
        )
        .await
        .expect("the watchdog must terminate a hung child");
        match exit {
            CrawlExit::Stalled { idle, limit } => {
                assert!(idle >= 900, "idle {idle}");
                assert_eq!(limit, 900);
            }
            _ => panic!("expected a stall, got a different exit"),
        }
        assert!(
            child.try_wait().expect("reap").is_some(),
            "the hung child was still running after the watchdog fired"
        );
    }

    #[test]
    fn a_lone_cover_is_not_crawl_output() {
        let root = scratch("cover-only");
        std::fs::write(root.join("cover.jpg"), b"fixture").expect("write cover");
        assert!(!crawl_has_output(&root, None, Engine::Webnovel));

        std::fs::create_dir_all(root.join("chapters")).expect("create chapters");
        std::fs::write(root.join("chapters/000001.zst"), b"fixture").expect("write chapter");
        assert!(crawl_has_output(&root, None, Engine::Webnovel));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_epub_or_work_dir_chapter_counts_as_output() {
        let root = scratch("epub-only");
        std::fs::write(root.join("Book.epub"), b"fixture").expect("write epub");
        assert!(crawl_has_output(&root, None, Engine::Webnovel));
        let _ = std::fs::remove_dir_all(&root);

        let destination = scratch("work-destination");
        let work = scratch("work-store");
        let chapters = work.join("novels/1/chapters");
        std::fs::create_dir_all(&chapters).expect("create chapters");
        std::fs::write(chapters.join("000001.zst"), b"fixture").expect("write chapter");
        assert!(crawl_has_output(
            &destination,
            Some(&work),
            Engine::Webnovel
        ));
        let _ = std::fs::remove_dir_all(&destination);
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn zero_output_message_keeps_a_precise_stdout_reason() {
        assert_eq!(
            zero_output_message(Some(
                "Source error · lncrawl.ServerError: No crawler found for the domain".to_string()
            )),
            "Source error · lncrawl.ServerError: No crawler found for the domain"
        );
        assert_eq!(
            zero_output_message(Some("MangaDex licensing: gap".to_string())),
            "MangaDex licensing: gap No usable chapter was produced."
        );
        assert_eq!(
            zero_output_message(None),
            "Process finished without producing a usable output file."
        );
    }

    #[test]
    fn expected_pace_rounds_up_and_needs_a_measurement() {
        assert_eq!(expected_seconds_for(20, 10.0), 120);
        assert_eq!(expected_seconds_for(7, 2.0), 210);
        assert_eq!(expected_seconds_for(1, 3.0), 20);
        assert_eq!(expected_seconds_for(0, 10.0), 0);
        assert_eq!(expected_seconds_for(10, 0.0), 0);
        assert_eq!(expected_seconds_for(10, f64::NAN), 0);
    }

    #[test]
    fn stdout_traceback_names_the_actionable_reason() {
        let mut tail = ErrorTail::default();
        tail.push_stdout("Traceback (most recent call last):");
        tail.push_stdout("  File \"/tmp/lncrawl/__main__.py\", line 40, in <module>");
        tail.push_stdout("    run()");
        tail.push_stdout(
            "lncrawl._server_errors.ServerError: Error(501): No crawler found for the domain",
        );
        let summary = tail.summary().expect("stdout failure must be summarised");
        assert!(summary.contains("No crawler found"), "{summary}");
        assert!(!summary.starts_with("Traceback"), "{summary}");
        assert!(!tail.is_transient(), "no crawler must stay fatal");
    }

    #[test]
    fn stdout_progress_and_info_lines_are_not_errors() {
        let mut tail = ErrorTail::default();
        tail.push_stdout("[21:00:01] INFO     Downloaded 12 chapters");
        tail.push_stdout("[21:00:02] INFO     0 errors");
        assert!(tail.summary().is_none());
    }

    #[test]
    fn stdout_exhausted_line_is_still_a_transient_failure() {
        let mut tail = ErrorTail::default();
        tail.push_stdout("Exhausted: no detection layer succeeded — transport failure (Timeout)");
        assert!(tail.is_transient());
    }

    #[test]
    fn transient_tails_retry_but_bad_urls_do_not() {
        let mut timeout = ErrorTail::default();
        timeout.push(
            "Exhausted: HTTPSConnectionPool(host='example.org', port=443): Max retries exceeded",
        );
        assert!(timeout.is_transient());

        let mut blocked = ErrorTail::default();
        blocked.push("HTTPError: 503 Service Unavailable from Cloudflare (Ray 1015)");
        assert!(blocked.is_transient());

        let mut bad_url = ErrorTail::default();
        bad_url.push("Unsupported URL: https://example.com/not-a-novel");
        assert!(!bad_url.is_transient());

        let mut missing = ErrorTail::default();
        missing.push("No crawler found for https://example.com/x (404)");
        assert!(!missing.is_transient());

        assert!(!ErrorTail::default().is_transient());
    }

    #[test]
    fn retry_is_webnovel_only_single_and_matches_the_failure_kind() {
        // A transient failure with nothing on disk starts over.
        assert_eq!(
            retry_action(Engine::Webnovel, 1, TaskStatus::Failed, false, false, true),
            Some(RetryKind::Restart)
        );
        // A stall resumes, with or without partial output.
        assert_eq!(
            retry_action(Engine::Webnovel, 1, TaskStatus::Failed, true, true, false),
            Some(RetryKind::Resume)
        );
        assert_eq!(
            retry_action(Engine::Webnovel, 1, TaskStatus::Failed, false, true, false),
            Some(RetryKind::Resume)
        );
        // A transient failure after real output is not restarted from zero.
        assert_eq!(
            retry_action(Engine::Webnovel, 1, TaskStatus::Failed, true, false, true),
            None
        );
        // Only one attempt.
        assert_eq!(
            retry_action(Engine::Webnovel, 2, TaskStatus::Failed, false, true, true),
            None
        );
        // Only webnovel jobs.
        assert_eq!(
            retry_action(Engine::Manga, 1, TaskStatus::Failed, false, true, true),
            None
        );
        // A completed job is never failed/retried.
        assert_eq!(
            retry_action(
                Engine::Webnovel,
                1,
                TaskStatus::Completed,
                false,
                true,
                true
            ),
            None
        );
        // Unknown failures fail closed.
        assert_eq!(
            retry_action(Engine::Webnovel, 1, TaskStatus::Failed, false, false, false),
            None
        );
    }
}

#[cfg(test)]
mod journal_tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "novelforge-journal-{label}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn task(id: &str, status: TaskStatus) -> DownloadTask {
        DownloadTask {
            id: id.to_string(),
            title: "Journal test".to_string(),
            query: "journal test".to_string(),
            engine: Engine::Webnovel,
            status,
            phase: TaskPhase::DownloadingChapters,
            created_at: now_secs(),
            destination: "/tmp/journal-test".to_string(),
            progress: 40.0,
            bytes: 1024,
            speed_bps: 12.0,
            eta_seconds: Some(30),
            elapsed_seconds: 5,
            idle_seconds: 1,
            expected_chapters_per_minute: None,
            expected_seconds: None,
            message: None,
            files: Vec::new(),
        }
    }

    #[test]
    fn a_running_job_is_restored_as_interrupted() {
        let root = scratch("interrupted");
        let config = root.join("settings.json");
        let state = AppState::new(Settings::default(), config.clone());
        state.jobs.lock().unwrap().insert(
            "job-1".to_string(),
            JobRecord {
                task: task("job-1", TaskStatus::Running),
                pid: 0,
                cancel: None,
            },
        );
        state.persist_journal();
        drop(state);

        let restored = AppState::new(Settings::default(), config);
        restored.restore_journal();
        let tasks = list_tasks(&restored);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, TaskStatus::Interrupted);
        assert_eq!(tasks[0].speed_bps, 0.0);
        assert!(tasks[0].message.as_deref().unwrap_or("").contains("Resume"));

        // The rewrite must land on disk immediately, not only at shutdown.
        let text = std::fs::read_to_string(restored.journal.as_ref()).unwrap();
        assert!(
            text.contains("\"interrupted\""),
            "journal still claims a live task: {text}"
        );
        assert!(!text.contains("\"running\""), "journal still says running");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn completed_jobs_keep_their_status_across_a_restart() {
        let root = scratch("completed");
        let config = root.join("settings.json");
        let state = AppState::new(Settings::default(), config.clone());
        state.jobs.lock().unwrap().insert(
            "job-2".to_string(),
            JobRecord {
                task: task("job-2", TaskStatus::Completed),
                pid: 0,
                cancel: None,
            },
        );
        state.persist_journal();
        drop(state);

        let restored = AppState::new(Settings::default(), config);
        restored.restore_journal();
        let tasks = list_tasks(&restored);
        assert_eq!(tasks[0].status, TaskStatus::Completed);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_journal_is_schema_stamped() {
        let root = scratch("schema");
        let state = AppState::new(Settings::default(), root.join("settings.json"));
        state.persist_journal();
        let text = std::fs::read_to_string(state.journal.as_ref()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["schemaVersion"], crate::model::SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(&root);
    }
}
