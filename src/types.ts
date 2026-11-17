export type Engine = "webnovel" | "manga" | "books";

export type BookSource =
  | "auto"
  | "libgen"
  | "welib"
  | "annas"
  | "ocean"
  | "gutenberg"
  | "standardebooks";

export type TaskStatus =
  | "queued"
  | "running"
  | "paused"
  | "verifying"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted";

export type TaskPhase =
  | "resolving"
  | "fetchingDetails"
  | "downloadingChapters"
  | "downloadingImages"
  | "packing"
  | "importing"
  | "fetchingIndex"
  | "auditing"
  | "retrying"
  | "exporting"
  | "verifying"
  | "completed";

type LogStream = "stdout" | "stderr" | "system";

export interface VerificationReport {
  ok: boolean;
  container: boolean;
  mimetype: string | null;
  ocfCompliant: boolean;
  detail: string;
  checkedAt: number;
  chaptersExpected?: number | null;
  chaptersFound?: number;
  emptyChapters?: number[];
  missingChapters?: number[];
  hasCover?: boolean;
}

interface OutputFile {
  path: string;
  name: string;
  size: number;
  kind: string;
  verified: VerificationReport | null;
}

export interface DownloadTask {
  id: string;
  title: string;
  query: string;
  engine: Engine;
  status: TaskStatus;
  phase: TaskPhase;
  createdAt: number;
  destination: string;
  progress: number;
  bytes: number;
  speedBps: number;
  etaSeconds: number | null;
  elapsedSeconds: number;
  idleSeconds: number;
  expectedChaptersPerMinute?: number | null;
  expectedSeconds?: number | null;
  message: string | null;
  files: OutputFile[];
}

/** Measured crawl cost saved beside a structured webnovel export. */
interface CrawlPace {
  chapters: number;
  seconds: number;
  secondsPerChapter: number;
  chaptersPerMinute: number;
  measuredAt: number;
}

export interface DownloadOptions {
  chapters?: string | null;
  jobs?: number | null;
  throttle?: boolean | null;
  bookSource?: BookSource | null;
  bookTitle?: string | null;
  bookAuthor?: string | null;
  bookIsbn?: string | null;
  bookSeries?: string | null;
  bookSeriesPosition?: number | null;
  duplicate?: "update" | "copy" | null;
  targetFolder?: string | null;
}

export interface ExistingDownload {
  folder: string;
  title: string;
  sourceUrl: string;
  chapterCount: number;
  downloadedChapters: number;
  complete: boolean;
  modified: number;
  epubPath: string | null;
  matches: number;
}

export interface NovelResult {
  title: string;
  url: string;
  source: string;
  detail: string | null;
}

export interface SourceSearchFailure {
  source: string;
  label: string;
  reason: string;
}

export interface SearchResponse {
  query: string;
  results: NovelResult[];
  searched: string[];
  failures: SourceSearchFailure[];
  cached: boolean;
  cacheAgeSeconds: number | null;
}

export interface BookCandidate {
  id: string;
  title: string;
  authors: string[];
  year: number | null;
  isbn: string | null;
  publisher: string | null;
  language: string | null;
  series: string | null;
  seriesPosition: number | null;
  coverUrl: string | null;
  source: string;
  url: string | null;
  releaseDate: string | null;
  released: boolean;
}

interface BookSeries {
  id: string;
  name: string;
  author: string | null;
  source: string;
  books: BookCandidate[];
}

export interface BookSearchResponse {
  query: string;
  candidates: BookCandidate[];
  series: BookSeries | null;
  searched: string[];
  failures: SourceSearchFailure[];
  cached: boolean;
  cacheAgeSeconds: number | null;
  hardcoverConfigured: boolean;
}

export interface HardcoverStatus {
  configured: boolean;
  path: string;
  detail: string | null;
}

export interface CatalogueSource {
  url: string;
  host: string;
  language: string;
  originLanguages: string[];
  translatedFromKorean: boolean;
  availabilityNote: string | null;
  search: boolean;
  manga: boolean;
  mtl: boolean;
  novelforgeId: string | null;
  supportLevel: string;
  downloadReady: boolean;
}

export interface ResolvedTitle {
  input: string;
  title: string;
  source: string;
  detail: string;
  confidence: number;
  matchedBy: "url" | "title" | "alias" | "author" | string;
  searched: string[];
  alternatives: NovelResult[];
  cached: boolean;
  cacheAgeSeconds: number | null;
}

export interface MangaSource {
  name: string;
  url: string;
  language: string;
  tags: string[];
}

export interface MangaSources {
  sources: MangaSource[];
  fetchedAt: number;
}

export interface DownloadRequest {
  engine: Engine;
  query: string;
  title?: string | null;
  options?: DownloadOptions;
}

export interface LogEvent {
  jobId: string;
  stream: LogStream;
  line: string;
  ts: number;
}

export interface ProgressEvent {
  jobId: string;
  percent: number;
  bytes: number;
  speedBps: number;
  etaSeconds: number | null;
  phase: TaskPhase;
  elapsedSeconds: number;
  idleSeconds: number;
  status: TaskStatus;
}

export interface StatusEvent {
  task: DownloadTask;
}

export interface ToolStatus {
  id: string;
  label: string;
  found: boolean;
  path: string | null;
  version: string | null;
  detail: string | null;
  hint: string;
}

export interface ToolsReport {
  lncrawl: ToolStatus;
  galleryDl: ToolStatus;
  aget: ToolStatus;
  allReady: boolean;
  checkedAt: number;
  binDir: string | null;
}

interface SourceLimit {
  host: string;
  concurrency: number;
  requestGapMs: number;
  chapterRate: number;
}

export interface Settings {
  downloadDir: string;
  lncrawlPath: string | null;
  galleryDlPath: string | null;
  agetPath: string | null;
  defaultEngine: Engine;
  mangaFormat: "cbz" | "zip";
  mangaThrottle: boolean;
  mangaLanguage: string | null;
  bookLanguage: string | null;
  sourceLimits: SourceLimit[];
  maxLibraryBytes: number;
  cleanupWorkAfterImport: boolean;
}

interface TransportHostReport {
  host: string;
  profile: string;
  label: string;
  safeConcurrency: number;
  maxConcurrency: number;
  currentConcurrency: number;
  requestGapMs: number;
  chapterRate: number;
  requests: number;
  retries: number;
  status429: number;
  status503: number;
  blocked: number;
  curlRequests: number;
  curlFallbacks: number;
  avgLatencyMs: number;
  note: string;
}

export interface TransportReport {
  hosts: TransportHostReport[];
}

type SourceSupportLevel =
  | "verified"
  | "index-verified"
  | "metadata-only"
  | "unverified";

export interface SourceProfileSummary {
  id: string;
  displayName: string;
  hosts: string[];
  language: string;
  originLanguages: string[];
  availabilityNote: string | null;
  support: SourceSupportLevel;
  supportLabel: string;
  downloadReady: boolean;
  search: boolean;
  index: boolean;
  evidence: string;
}

interface LibraryEntry {
  name: string;
  path: string;
  size: number;
  modified: number;
  kind: string;
  folder: string;
}

interface LibraryStats {
  count: number;
  totalBytes: number;
  byKind: Record<string, number>;
}

export interface NovelMetadata {
  title: string;
  author: string | null;
  authors: string[];
  synopsis: string | null;
  tags: string[];
  status: string | null;
  language: string | null;
  chapterCount: number | null;
  sourceUrl: string;
  source: string;
  cover: string | null;
  aliases: string[];
}

interface NovelChapter {
  number: number;
  title: string;
  url: string;
  volume: number | null;
  downloaded: boolean;
  file: string | null;
}

export interface NovelChapterIndex {
  sourceUrl: string;
  source: string;
  title: string | null;
  total: number;
  complete: boolean;
  detail: string | null;
  chapters: NovelChapter[];
  cached: boolean;
  cacheAgeSeconds: number | null;
  transport: string | null;
  titleSource: string | null;
}

export interface LibraryNovel {
  folder: string;
  title: string;
  author: string | null;
  synopsis: string | null;
  tags: string[];
  cover: string | null;
  chapterCount: number;
  downloadedChapters: number;
  complete: boolean;
  sourceUrl: string | null;
  modified: number;
  epubPath: string | null;
  retryable: boolean;
  provenance: string | null;
  diskBytes: number;
  sourceHost: string | null;
  sourceRank: number;
  crawl: CrawlPace | null;
}

/** A measured-cheaper host that verified the same title (NF-041). */
interface FasterSource {
  title: string;
  url: string;
  host: string;
  detail: string;
  confidence: number;
  currentHost: string | null;
  currentRank: number;
  candidateRank: number;
}

export interface FasterSourceReport {
  alternative: FasterSource | null;
  note: string;
  currentHost: string | null;
  currentRank: number;
}

export interface LibraryStorage {
  libraryBytes: number;
  partialBytes: number;
  partialFolders: number;
  workBytes: number;
  workFolders: number;
  quotaBytes: number;
  overQuota: boolean;
}

export interface CleanupReport {
  removed: string[];
  kept: string[];
  errors: string[];
  bytesFreed: number;
}

export interface LibraryArchive {
  name: string;
  path: string;
  folder: string;
  kind: string;
  size: number;
  modified: number;
  adoptable: boolean;
  reason: string;
}

export interface DownloadDetails {
  folder: string;
  title: string;
  sourceUrl: string | null;
  chapterCount: number;
  downloadedChapters: number;
  selection: string | null;
  indexComplete: boolean;
  indexDetail: string | null;
  complete: boolean;
  chapters: NovelChapter[];
}

export interface LibraryResponse {
  entries: LibraryEntry[];
  stats: LibraryStats;
  novels?: LibraryNovel[];
  archives?: LibraryArchive[];
  storage?: LibraryStorage;
}

export interface AppInfo {
  name: string;
  version: string;
  commit: string;
  tauriVersion: string;
  platform: string;
  arch: string;
  downloadDir: string;
}

export type Page =
  | "home"
  | "webnovels"
  | "manga"
  | "books"
  | "library"
  | "downloads"
  | "settings"
  | "guardrails"
  | "about";

export interface EngineMeta {
  id: Engine;
  label: string;
  short: string;
  tool: string;
  hint: string;
  placeholder: string;
  format: string;
}

export const ENGINES: Record<Engine, EngineMeta> = {
  webnovel: {
    id: "webnovel",
    label: "Webnovels",
    short: "Webnovel",
    tool: "lncrawl",
    hint: "Search a title or synonym — or paste a page URL",
    placeholder: "reverend insanity · lotm · https://novelfire.net/book/…",
    format: "EPUB",
  },
  manga: {
    id: "manga",
    label: "Manga / Manhwa",
    short: "Manga",
    tool: "gallery-dl",
    hint: "Paste a page URL from any gallery-dl manga extractor",
    placeholder: "https://mangadex.org/title/…",
    format: "CBZ",
  },
  books: {
    id: "books",
    label: "EPUB Books",
    short: "Book",
    tool: "aget",
    hint: "WeLib → Anna’s Archive → OceanOfPDF",
    placeholder: "The Left Hand of Darkness · Dune · 三体",
    format: "EPUB",
  },
};
