import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppInfo,
  BookSearchResponse,
  CatalogueSource,
  CleanupReport,
  DownloadRequest,
  DownloadTask,
  Engine,
  ExistingDownload,
  DownloadDetails,
  FasterSourceReport,
  HardcoverStatus,
  LibraryNovel,
  LibraryResponse,
  LogEvent,
  MangaSources,
  ProgressEvent,
  ResolvedTitle,
  SearchResponse,
  Settings,
  SourceProfileSummary,
  StatusEvent,
  ToolsReport,
  TransportReport,
  VerificationReport,
  NovelChapterIndex,
  NovelMetadata,
} from "@/types";

export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function invokeCommand<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isTauriRuntime()) {
    throw new Error(
      "NovelForge needs the desktop runtime. Launch it with `npm run tauri dev`.",
    );
  }
  return invoke<T>(command, args);
}

function subscribe<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  if (!isTauriRuntime()) {
    return Promise.resolve(() => undefined);
  }
  return listen<T>(event, (payload) => handler(payload.payload));
}

export const api = {
  checkTools: () => invokeCommand<ToolsReport>("check_tools"),
  resolveTitle: (
    engine: Engine,
    query: string,
    mode: "auto" | "author" = "auto",
    sources: string[] = [],
  ) =>
    invokeCommand<ResolvedTitle>("resolve_title", {
      engine,
      query,
      mode,
      sources,
    }),
  searchWebnovels: (query: string, sources: string[] = []) =>
    invokeCommand<SearchResponse>("search_webnovels", { query, sources }),
  searchBooks: (query: string, mode: "auto" | "books" | "series" = "auto") =>
    invokeCommand<BookSearchResponse>("search_books", { query, mode }),
  getHardcoverStatus: () =>
    invokeCommand<HardcoverStatus>("get_hardcover_status"),
  setHardcoverToken: (token: string) =>
    invokeCommand<HardcoverStatus>("set_hardcover_token", { token }),
  clearHardcoverToken: () =>
    invokeCommand<HardcoverStatus>("clear_hardcover_token"),
  listSourceCatalogue: () =>
    invokeCommand<CatalogueSource[]>("list_source_catalogue"),
  fetchNovelMetadata: (url: string) =>
    invokeCommand<NovelMetadata>("fetch_novel_metadata", { url }),
  fetchNovelChapters: (url: string, refresh = false) =>
    invokeCommand<NovelChapterIndex>("fetch_novel_chapters", { url, refresh }),
  findExistingDownload: (url: string) =>
    invokeCommand<ExistingDownload | null>("find_existing_download", { url }),
  listMangaSources: () => invokeCommand<MangaSources>("list_manga_sources"),
  startDownload: (request: DownloadRequest) =>
    invokeCommand<DownloadTask>("start_download", { request }),
  pauseDownload: (jobId: string) =>
    invokeCommand<DownloadTask>("pause_download", { jobId }),
  resumeDownload: (jobId: string) =>
    invokeCommand<DownloadTask>("resume_download", { jobId }),
  resumeInterrupted: (jobId: string) =>
    invokeCommand<DownloadTask>("resume_interrupted", { jobId }),
  cancelDownload: (jobId: string) =>
    invokeCommand<DownloadTask>("cancel_download", { jobId }),
  clearFinished: () => invokeCommand<DownloadTask[]>("clear_finished"),
  getTasks: () => invokeCommand<DownloadTask[]>("get_tasks"),
  getSettings: () => invokeCommand<Settings>("get_settings"),
  saveSettings: (settings: Settings) =>
    invokeCommand<Settings>("save_settings", { settings }),
  getTransportReport: () => invokeCommand<TransportReport>("get_transport_report"),
  listSourceProfiles: () =>
    invokeCommand<SourceProfileSummary[]>("list_source_profiles"),
  listLibrary: () => invokeCommand<LibraryResponse>("list_library"),
  adoptArchive: (path: string) =>
    invokeCommand<LibraryNovel>("adopt_archive", { path }),
  getDownloadDetails: (folder: string) =>
    invokeCommand<DownloadDetails>("get_download_details", { folder }),
  deleteDownload: (folder: string) =>
    invokeCommand<void>("delete_download", { folder }),
  cleanupEngineWork: () => invokeCommand<CleanupReport>("cleanup_engine_work"),
  deletePartialDownloads: () =>
    invokeCommand<CleanupReport>("delete_partial_downloads"),
  retryDownload: (folder: string, refresh = false) =>
    invokeCommand<DownloadTask>("retry_download", { folder, refresh }),
  updateDownload: (folder: string, refresh = false) =>
    invokeCommand<DownloadTask>("update_download", { folder, refresh }),
  suggestFasterSource: (folder: string) =>
    invokeCommand<FasterSourceReport>("suggest_faster_source", { folder }),
  openChapters: (folder: string) =>
    invokeCommand<void>("open_chapters", { folder }),
  verifyPath: (path: string) =>
    invokeCommand<VerificationReport>("verify_path", { path }),
  revealPath: (path: string) =>
    invokeCommand<void>("reveal_path", { path }),
  openDownloads: () => invokeCommand<void>("open_downloads"),
  getAppInfo: () => invokeCommand<AppInfo>("get_app_info"),
  getDiagnostics: () => invokeCommand<string>("get_diagnostics"),
  confirmQuit: () => invokeCommand<void>("confirm_quit"),

  onLog: (handler: (event: LogEvent) => void) =>
    subscribe<LogEvent>("download://log", handler),
  onProgress: (handler: (event: ProgressEvent) => void) =>
    subscribe<ProgressEvent>("download://progress", handler),
  onStatus: (handler: (event: StatusEvent) => void) =>
    subscribe<StatusEvent>("download://status", handler),
  onLibraryChanged: (handler: () => void) =>
    subscribe<number>("library://changed", handler),
  onQuitRequested: (handler: () => void) =>
    subscribe<null>("app://quit-requested", handler),
};
