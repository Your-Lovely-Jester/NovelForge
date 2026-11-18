use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The three download engines the app can route a job to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    #[default]
    Webnovel,
    Manga,
    Books,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Self::Webnovel => "Webnovel",
            Self::Manga => "Manga",
            Self::Books => "Books",
        }
    }

    pub fn tool_id(self) -> &'static str {
        match self {
            Self::Webnovel => "lncrawl",
            Self::Manga => "gallery_dl",
            Self::Books => "aget",
        }
    }
}

/// Version written into `metadata.json`, `index.json` and `settings.json`.
/// Readers must tolerate a newer value and migrate forward in place.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    #[default]
    Queued,
    Running,
    Paused,
    Verifying,
    Completed,
    Failed,
    Cancelled,
    /// The app closed while this job was still active. Restored from the job
    /// journal on the next launch; the user can resume it from Downloads.
    Interrupted,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

/// What to do when a structured folder already exists for the same source URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DuplicateMode {
    /// Reuse the existing folder, overwriting re-downloaded chapters while
    /// preserving chapters that are not part of this run.
    Update,
    /// Keep the existing folder and create an explicit `-copy-N` edition.
    Copy,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadOptions {
    pub chapters: Option<String>,
    pub jobs: Option<u8>,
    pub throttle: Option<bool>,
    /// Book source order: `auto`, `libgen`, `welib`, `annas` or `ocean`.
    pub book_source: Option<String>,
    /// Verified catalogue record handed to `aget`. When one of these is set,
    /// `aget` scores every file row and fails closed instead of taking the
    /// first search hit.
    pub book_title: Option<String>,
    pub book_author: Option<String>,
    pub book_isbn: Option<String>,
    /// Series name and catalogue position. When set on a Books job the book is
    /// shelved as `<Author>/<Series>/<NN> - <Title>/`.
    pub book_series: Option<String>,
    pub book_series_position: Option<f64>,
    /// None means "start a new download"; an existing folder for the same
    /// source then fails instead of silently creating a duplicate.
    pub duplicate: Option<DuplicateMode>,
    /// Runtime-only target used by library actions. When present the job must
    /// update this exact folder instead of choosing a match by source URL.
    #[serde(default)]
    pub target_folder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NovelResult {
    pub title: String,
    pub url: String,
    pub source: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<NovelResult>,
    /// Source URLs that were actually queried.
    pub searched: Vec<String>,
    /// Per-source failures. A source with zero hits but no failure is not
    /// listed here; the UI reports it as searched-and-empty.
    pub failures: Vec<SourceSearchFailure>,
    /// True when at least one source answered from the short-lived disk cache.
    pub cached: bool,
    /// Age of the oldest cached source answer, in seconds.
    pub cache_age_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSearchFailure {
    pub source: String,
    pub label: String,
    pub reason: String,
}

/// Real metadata scraped from a source detail page. `cover` is a data URL so
/// the renderer can show it without granting filesystem access to the webview.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NovelMetadata {
    pub title: String,
    pub author: Option<String>,
    pub authors: Vec<String>,
    pub synopsis: Option<String>,
    pub tags: Vec<String>,
    pub status: Option<String>,
    pub language: Option<String>,
    pub chapter_count: Option<u32>,
    pub source_url: String,
    pub source: String,
    pub cover: Option<String>,
    /// Alternative titles advertised by the source page (JSON-LD
    /// `alternateName` or an “Alternative names” row). Used by the resolver to
    /// match a synonym to a source-native title.
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NovelChapter {
    pub number: u32,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub volume: Option<u32>,
    #[serde(default)]
    pub downloaded: bool,
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NovelChapterIndex {
    pub source_url: String,
    pub source: String,
    pub title: Option<String>,
    pub total: u32,
    /// False when the source advertised more chapters than the walker could
    /// list, or when the TOC could not be fetched in full.
    pub complete: bool,
    /// Human-readable evidence for the completeness decision. Always present
    /// for generic sources, and copied into `metadata.json` as `indexDetail`.
    #[serde(default)]
    pub detail: Option<String>,
    pub chapters: Vec<NovelChapter>,
    /// Runtime-only: true when this response was served from the index cache.
    #[serde(default)]
    pub cached: bool,
    /// Runtime-only: age of the cached response in seconds.
    #[serde(default)]
    pub cache_age_seconds: Option<u64>,
    /// Runtime-only: per-host request/429/retry summary for this fetch.
    #[serde(default)]
    pub transport: Option<String>,
    /// Where borrowed chapter names came from when this source's table of
    /// contents left some rows as bare numbers. `None` when every row was
    /// already named by the source, or when no verified donor matched.
    #[serde(default)]
    pub title_source: Option<String>,
    /// True once a donor lookup has been attempted for this index, so a source
    /// with no titled counterpart is not searched again on every cache hit.
    #[serde(default)]
    pub title_lookup_attempted: bool,
    /// True once the per-page name recovery pass has completed for this index.
    /// Kept separate from `title_lookup_attempted` so a recovery that ran out
    /// of budget can resume from the rows that remain numbered.
    #[serde(default)]
    pub title_recovery_attempted: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportVerification {
    pub ok: bool,
    pub chapters_expected: Option<u32>,
    pub chapters_found: u32,
    pub empty_chapters: Vec<u32>,
    pub missing_chapters: Vec<u32>,
    pub has_cover: bool,
    pub detail: String,
}

/// Measured crawl cost for one webnovel run. Written from the run's own
/// timings, never estimated from the current network, so the Library can say
/// what a source actually did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CrawlPace {
    /// Chapters written by the run the measurement came from.
    pub chapters: u32,
    /// Crawl wall time for that run, including a resumed attempt.
    pub seconds: f64,
    pub seconds_per_chapter: f64,
    pub chapters_per_minute: f64,
    /// Unix seconds when the measurement was taken.
    pub measured_at: u64,
}

/// The `metadata.json` written beside every structured download.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportMetadata {
    pub title: String,
    pub author: Option<String>,
    pub authors: Vec<String>,
    pub synopsis: Option<String>,
    pub tags: Vec<String>,
    pub status: Option<String>,
    pub language: Option<String>,
    pub source_url: Option<String>,
    pub source: Option<String>,
    pub cover: Option<String>,
    pub epub: Option<String>,
    pub chapter_count: u32,
    pub downloaded_chapters: u32,
    /// The chapter selection this run was asked to download, if any. Retry
    /// uses it so a partial selection never expands into the whole novel.
    #[serde(default)]
    pub selection: Option<String>,
    pub index_complete: bool,
    /// Why the chapter index was accepted or rejected. Written for both
    /// complete and incomplete indexes so a partial download is explainable.
    #[serde(default)]
    pub index_detail: Option<String>,
    /// True only when the export passed chapter verification and, for
    /// webnovels, the full source index was retrieved.
    #[serde(default)]
    pub complete: bool,
    /// How this structured download entered the library. `None` for native
    /// downloads and `Some("migrated")` for old archive folders adopted in
    /// place without touching the original file.
    #[serde(default)]
    pub provenance: Option<String>,
    /// Measured crawl pace from the run that produced this export. Absent for
    /// books, manga, adopted archives and folders written before NF-041.
    #[serde(default)]
    pub crawl: Option<CrawlPace>,
    pub chapters: Vec<NovelChapter>,
    pub generated_at: u64,
    pub verified: ExportVerification,
}

/// A structured download surfaced on the dashboard and Home shelf.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryNovel {
    pub folder: String,
    pub title: String,
    pub author: Option<String>,
    pub synopsis: Option<String>,
    pub tags: Vec<String>,
    pub cover: Option<String>,
    pub chapter_count: u32,
    pub downloaded_chapters: u32,
    pub complete: bool,
    pub source_url: Option<String>,
    pub modified: u64,
    pub epub_path: Option<String>,
    /// Packed CBZ/ZIP for manga folders, when one exists beside the chapter
    /// tree. `None` for novels (their EPUB is `epub_path`).
    #[serde(default)]
    pub archive_path: Option<String>,
    /// Number of per-chapter archives under `chapters/`. `0` means this folder
    /// has no chapter archives (a legacy single archive, or none at all).
    #[serde(default)]
    pub archive_count: u32,
    /// True when this folder can be retried/updated through the webnovel
    /// pipeline (structured EPUB with an http(s) source URL).
    pub retryable: bool,
    /// `Some("migrated")` when this card was created by adopting a loose
    /// archive that predates the structured library.
    #[serde(default)]
    pub provenance: Option<String>,
    /// Bytes this folder occupies on disk, including chapter files, images,
    /// the cover and the bundled archive. Measured at scan time.
    #[serde(default)]
    pub disk_bytes: u64,
    /// Host of the stored source URL, e.g. `novelfire.net`.
    #[serde(default)]
    pub source_host: Option<String>,
    /// Registry preference rank for that host (lower is measured cheaper).
    /// `u8::MAX` means the host is unknown to the registry.
    #[serde(default)]
    pub source_rank: u8,
    /// Measured crawl pace from the last run, when this folder has one.
    #[serde(default)]
    pub crawl: Option<CrawlPace>,
}

/// An archive file that does not belong to a structured download yet.
///
/// Loose folders from older NovelForge versions keep their EPUB but have no
/// `metadata.json`, so they never appeared as novels. They are surfaced here
/// with an explicit adoptability decision instead of being silently ignored.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryArchive {
    pub name: String,
    pub path: String,
    pub folder: String,
    pub kind: String,
    pub size: u64,
    pub modified: u64,
    /// True when one click can create structured sidecars beside the file.
    pub adoptable: bool,
    /// Why the file is or is not adoptable. Always populated.
    pub reason: String,
}

/// The chapter index and state for one structured download, loaded on demand
/// when the library opens its index viewer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDetails {
    pub folder: String,
    pub title: String,
    pub source_url: Option<String>,
    pub chapter_count: u32,
    pub downloaded_chapters: u32,
    pub selection: Option<String>,
    pub index_complete: bool,
    pub index_detail: Option<String>,
    pub complete: bool,
    pub chapters: Vec<NovelChapter>,
}

/// An existing structured download that matches the requested source URL.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingDownload {
    pub folder: String,
    pub title: String,
    pub source_url: String,
    pub chapter_count: u32,
    pub downloaded_chapters: u32,
    pub complete: bool,
    pub modified: u64,
    pub epub_path: Option<String>,
    /// How many folders in the library match this source. `folder` points at
    /// the newest complete match (or the newest match when none is complete).
    pub matches: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTitle {
    /// The text the user typed.
    pub input: String,
    /// Canonical title used for the task and output directory.
    pub title: String,
    /// Direct source URL handed to the crawler.
    pub source: String,
    /// Which resolver stage produced the match.
    pub detail: String,
    /// `0.0–1.0` score for the chosen match. `1.0` for a direct URL.
    pub confidence: f32,
    /// `url`, `title`, `alias` or `author`.
    pub matched_by: String,
    /// Source hosts (and title databases) that were actually consulted.
    pub searched: Vec<String>,
    /// Other plausible source matches, best first. Empty for a direct URL.
    pub alternatives: Vec<NovelResult>,
    /// True when the search rows, author page or summary behind this match
    /// came from the short-lived disk cache.
    pub cached: bool,
    /// Age of the cached evidence, in seconds.
    pub cache_age_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaSource {
    pub name: String,
    pub url: String,
    pub language: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaSources {
    pub sources: Vec<MangaSource>,
    pub fetched_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequest {
    pub engine: Engine,
    pub query: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub options: DownloadOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationReport {
    pub ok: bool,
    pub container: bool,
    pub mimetype: Option<String>,
    pub ocf_compliant: bool,
    pub detail: String,
    pub checked_at: u64,
    #[serde(default)]
    pub chapters_expected: Option<u32>,
    #[serde(default)]
    pub chapters_found: u32,
    #[serde(default)]
    pub empty_chapters: Vec<u32>,
    #[serde(default)]
    pub missing_chapters: Vec<u32>,
    #[serde(default)]
    pub has_cover: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub kind: String,
    pub verified: Option<VerificationReport>,
}

/// The real stage a job is in. `status` describes lifecycle; `phase` describes
/// what the pipeline is doing inside that lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub enum TaskPhase {
    #[default]
    Resolving,
    FetchingDetails,
    DownloadingChapters,
    DownloadingImages,
    Packing,
    Importing,
    FetchingIndex,
    Auditing,
    Retrying,
    Exporting,
    Verifying,
    Completed,
}

impl TaskPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Resolving => "Resolving source",
            Self::FetchingDetails => "Fetching details",
            Self::DownloadingChapters => "Downloading chapters",
            Self::DownloadingImages => "Downloading images",
            Self::Packing => "Packing archive",
            Self::Importing => "Importing output",
            Self::FetchingIndex => "Fetching full index",
            Self::Auditing => "Auditing chapters",
            Self::Retrying => "Retrying chapters",
            Self::Exporting => "Exporting structured files",
            Self::Verifying => "Verifying output",
            Self::Completed => "Complete",
        }
    }

    /// Phases advance in this order while lncrawl is printing its own progress.
    /// Explicit pipeline steps can still move backwards (retry -> audit).
    pub fn rank(self) -> u8 {
        match self {
            Self::Resolving => 0,
            Self::FetchingDetails => 1,
            Self::DownloadingChapters => 2,
            Self::DownloadingImages => 3,
            Self::Packing => 4,
            Self::Importing => 5,
            Self::FetchingIndex => 6,
            Self::Auditing => 7,
            Self::Retrying => 8,
            Self::Exporting => 9,
            Self::Verifying => 10,
            Self::Completed => 11,
        }
    }

    pub fn is_transfer(self) -> bool {
        matches!(
            self,
            Self::DownloadingChapters | Self::DownloadingImages | Self::FetchingIndex
        )
    }

    /// True while the pipeline is still inside lncrawl's crawl stages.
    pub fn is_crawl(self) -> bool {
        matches!(
            self,
            Self::Resolving
                | Self::FetchingDetails
                | Self::DownloadingChapters
                | Self::DownloadingImages
                | Self::Packing
        )
    }

    /// True while bytes are written into the job sandbox rather than the
    /// crawler's work directory.
    pub fn measures_destination(self) -> bool {
        matches!(
            self,
            Self::Importing | Self::Exporting | Self::Verifying | Self::Completed
        )
    }

    /// Phases where a byte-rate reading is meaningful.
    pub fn shows_speed(self) -> bool {
        matches!(
            self,
            Self::DownloadingChapters
                | Self::DownloadingImages
                | Self::Packing
                | Self::Importing
                | Self::Exporting
                | Self::Verifying
        )
    }

    pub fn code(self) -> u8 {
        self.rank()
    }

    pub fn from_code(value: u8) -> Self {
        match value {
            1 => Self::FetchingDetails,
            2 => Self::DownloadingChapters,
            3 => Self::DownloadingImages,
            4 => Self::Packing,
            5 => Self::Importing,
            6 => Self::FetchingIndex,
            7 => Self::Auditing,
            8 => Self::Retrying,
            9 => Self::Exporting,
            10 => Self::Verifying,
            11 => Self::Completed,
            _ => Self::Resolving,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTask {
    pub id: String,
    pub title: String,
    pub query: String,
    pub engine: Engine,
    pub status: TaskStatus,
    #[serde(default)]
    pub phase: TaskPhase,
    pub created_at: u64,
    pub destination: String,
    pub progress: f32,
    pub bytes: u64,
    pub speed_bps: f64,
    pub eta_seconds: Option<u64>,
    /// Wall time minus time spent paused.
    #[serde(default)]
    pub elapsed_seconds: u64,
    /// Seconds since the last log line or measured write.
    #[serde(default)]
    pub idle_seconds: u64,
    /// Measured pace of the source this job is updating, when known.
    #[serde(default)]
    pub expected_chapters_per_minute: Option<f64>,
    /// Estimated crawl time for this job's selection at that measured pace.
    #[serde(default)]
    pub expected_seconds: Option<u64>,
    pub message: Option<String>,
    pub files: Vec<OutputFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEvent {
    pub job_id: String,
    pub stream: LogStream,
    pub line: String,
    pub ts: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub job_id: String,
    pub percent: f32,
    pub bytes: u64,
    pub speed_bps: f64,
    pub eta_seconds: Option<u64>,
    pub phase: TaskPhase,
    pub elapsed_seconds: u64,
    pub idle_seconds: u64,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusEvent {
    pub task: DownloadTask,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: u64,
    pub kind: String,
    pub folder: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    pub count: usize,
    pub total_bytes: u64,
    pub by_kind: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct LibraryStorage {
    /// Every non-hidden file under the download root (library + archives).
    pub library_bytes: u64,
    /// Bytes held by incomplete structured folders.
    pub partial_bytes: u64,
    pub partial_folders: u32,
    /// Bytes held by per-job `.novelforge-work` trees.
    pub work_bytes: u64,
    pub work_folders: u32,
    /// `0` means no quota is configured.
    pub quota_bytes: u64,
    pub over_quota: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryResponse {
    pub entries: Vec<LibraryEntry>,
    pub stats: LibraryStats,
    #[serde(default)]
    pub novels: Vec<LibraryNovel>,
    /// Loose archives outside structured downloads, including whether each can
    /// be migrated in place.
    #[serde(default)]
    pub archives: Vec<LibraryArchive>,
    #[serde(default)]
    pub storage: LibraryStorage,
}

/// Result of an explicit cleanup action. Nothing is removed silently: the
/// caller gets the exact folders touched, the bytes reclaimed, what was kept
/// because a job still owns it, and any per-folder failures.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    pub removed: Vec<String>,
    pub kept: Vec<String>,
    pub errors: Vec<String>,
    pub bytes_freed: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    /// Short commit the installer was built from; `dev` for local builds.
    /// Makes a stale installer obvious when diagnosing a Windows install.
    pub commit: String,
    pub tauri_version: String,
    pub platform: String,
    pub arch: String,
    pub download_dir: String,
}

pub const EVENT_LOG: &str = "download://log";
pub const EVENT_PROGRESS: &str = "download://progress";
pub const EVENT_STATUS: &str = "download://status";
pub const EVENT_LIBRARY: &str = "library://changed";
/// Emitted while a source's table of contents is walked page by page:
/// `{ url, page, pages }`.
pub const EVENT_INDEX_PROGRESS: &str = "novel://index-progress";
/// Emitted when the window close button is pressed while downloads are live,
/// so the frontend can show a confirmation dialog instead of losing the crawl.
pub const EVENT_QUIT_REQUESTED: &str = "app://quit-requested";
