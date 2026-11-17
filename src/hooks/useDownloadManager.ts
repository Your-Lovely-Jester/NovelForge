import { useCallback, useEffect, useMemo, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import type {
  AppInfo,
  CleanupReport,
  DownloadOptions,
  DownloadRequest,
  DownloadTask,
  DownloadDetails,
  Engine,
  LibraryResponse,
  LogEvent,
  MangaSources,
  ResolvedTitle,
  SearchResponse,
  Settings,
  ToolStatus,
  ToolsReport,
  VerificationReport,
} from "@/types";
import { api } from "@/lib/tauri";
import { formatBytes, isActive, isTerminal } from "@/lib/utils";

export interface Notice {
  id: number;
  kind: "info" | "success" | "error";
  title: string;
  message?: string;
}

const EMPTY_LIBRARY: LibraryResponse = {
  entries: [],
  stats: { count: 0, totalBytes: 0, byKind: {} },
};

export function useDownloadManager() {
  const [tasks, setTasks] = useState<DownloadTask[]>([]);
  const [logs, setLogs] = useState<LogEvent[]>([]);
  const [tools, setTools] = useState<ToolsReport | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [library, setLibrary] = useState<LibraryResponse>(EMPTY_LIBRARY);
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [starting, setStarting] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [activeJobId, setActiveJobId] = useState<string | null>(null);
  const [mangaSources, setMangaSources] = useState<MangaSources | null>(null);
  const [sourcesLoading, setSourcesLoading] = useState(false);
  const [resolving, setResolving] = useState(false);
  const [quitRequested, setQuitRequested] = useState(false);

  const showNotice = useCallback(
    (kind: Notice["kind"], title: string, message?: string) => {
      setNotice({ id: Date.now(), kind, title, message });
    },
    [],
  );

  const refreshLibrary = useCallback(async () => {
    try {
      const response = await api.listLibrary();
      setLibrary(response);
    } catch {
      // Library refresh is best-effort; the dashboard still works offline.
    }
  }, []);

  const refreshTools = useCallback(async () => {
    try {
      const report = await api.checkTools();
      setTools(report);
      return report;
    } catch {
      setTools(null);
      return null;
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    async function boot() {
      const [taskResult, toolResult, settingsResult, libraryResult, infoResult] =
        await Promise.allSettled([
          api.getTasks(),
          api.checkTools(),
          api.getSettings(),
          api.listLibrary(),
          api.getAppInfo(),
        ]);

      if (disposed) return;

      if (taskResult.status === "fulfilled") setTasks(taskResult.value);
      if (toolResult.status === "fulfilled") setTools(toolResult.value);
      if (settingsResult.status === "fulfilled") setSettings(settingsResult.value);
      if (libraryResult.status === "fulfilled") setLibrary(libraryResult.value);
      if (infoResult.status === "fulfilled") setAppInfo(infoResult.value);
      setLoading(false);
    }

    async function subscribe() {
      // Tauri's `listen` resolves asynchronously. Under React StrictMode the
      // effect can be torn down before a listener is registered, so an
      // untracked promise would leak a permanent second subscription.
      async function track(registration: Promise<() => void>) {
        const unlisten = await registration;
        if (disposed) {
          unlisten();
          return;
        }
        unlisteners.push(unlisten);
      }

      await track(
        api.onLog((event) => {
          setLogs((current) => {
            const next = current.length >= 2000
              ? current.slice(current.length - 1999)
              : current.slice();
            next.push(event);
            return next;
          });
        }),
      );
      await track(
        api.onProgress((event) => {
          setTasks((current) =>
            current.map((task) =>
              task.id === event.jobId
                ? {
                    ...task,
                    progress: event.percent,
                    bytes: event.bytes,
                    speedBps: event.speedBps,
                    etaSeconds: event.etaSeconds,
                    phase: event.phase,
                    elapsedSeconds: event.elapsedSeconds,
                    idleSeconds: event.idleSeconds,
                    status: event.status,
                  }
                : task,
            ),
          );
        }),
      );
      await track(
        api.onStatus((event) => {
          setTasks((current) => {
            const exists = current.some((task) => task.id === event.task.id);
            if (!exists) return [...current, event.task];
            return current.map((task) =>
              task.id === event.task.id ? event.task : task,
            );
          });
          if (isTerminal(event.task.status)) {
            void refreshLibrary();
          }
        }),
      );
      await track(
        api.onLibraryChanged(() => {
          void refreshLibrary();
        }),
      );

      await track(
        api.onQuitRequested(() => {
          setQuitRequested(true);
        }),
      );
    }

    void boot();
    void subscribe();

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [refreshLibrary]);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 5200);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const searchNovels = useCallback(
    async (query: string, sources: string[] = []): Promise<SearchResponse | null> => {
      const trimmed = query.trim();
      if (trimmed.length < 2) {
        showNotice("error", "Search needs a title", "Type at least two characters.");
        return null;
      }
      setResolving(true);
      try {
        return await api.searchWebnovels(trimmed, sources);
      } catch (error) {
        showNotice("error", "Search failed", errorMessage(error));
        return null;
      } finally {
        setResolving(false);
      }
    },
    [showNotice],
  );

  const resolveTitle = useCallback(
    async (
      engine: Engine,
      query: string,
      mode: "auto" | "author" = "auto",
      sources: string[] = [],
    ): Promise<ResolvedTitle | null> => {
      try {
        return await api.resolveTitle(engine, query, mode, sources);
      } catch (error) {
        showNotice("error", "Could not resolve source", errorMessage(error));
        return null;
      }
    },
    [showNotice],
  );

  const loadMangaSources = useCallback(async (): Promise<MangaSources | null> => {
    if (mangaSources) return mangaSources;
    setSourcesLoading(true);
    try {
      const report = await api.listMangaSources();
      setMangaSources(report);
      return report;
    } catch (error) {
      showNotice("error", "Could not read gallery-dl sources", errorMessage(error));
      return null;
    } finally {
      setSourcesLoading(false);
    }
  }, [mangaSources, showNotice]);

  const start = useCallback(
    async (
      engine: Engine,
      query: string,
      options?: DownloadOptions,
      title?: string,
    ): Promise<boolean> => {
      const trimmed = query.trim();
      if (!trimmed) {
        showNotice("error", "Nothing to download", "Enter a URL, title, or author first.");
        return false;
      }
      if (engine === "manga" && !/^https?:\/\//i.test(trimmed)) {
        showNotice(
          "error",
          "Direct page URL required",
          "gallery-dl needs a page URL from one of its supported manga sites.",
        );
        return false;
      }

      setStarting(true);
      try {
        const request: DownloadRequest = {
          engine,
          query: trimmed,
          options,
          title: title?.trim() || undefined,
        };
        const task = await api.startDownload(request);
        setTasks((current) =>
          current.some((existing) => existing.id === task.id)
            ? current
            : [...current, task],
        );
        setActiveJobId(task.id);
        showNotice(
          "success",
          "Download initialized",
          `${task.title} is now running in the background.`,
        );
        return true;
      } catch (error) {
        showNotice("error", "Could not start download", errorMessage(error));
        return false;
      } finally {
        setStarting(false);
      }
    },
    [showNotice],
  );

  const pause = useCallback(
    async (jobId: string) => {
      try {
        const task = await api.pauseDownload(jobId);
        mergeTask(setTasks, task);
      } catch (error) {
        showNotice("error", "Pause failed", errorMessage(error));
      }
    },
    [showNotice],
  );

  const resume = useCallback(
    async (jobId: string) => {
      try {
        const task = await api.resumeDownload(jobId);
        mergeTask(setTasks, task);
      } catch (error) {
        showNotice("error", "Resume failed", errorMessage(error));
      }
    },
    [showNotice],
  );

  const cancel = useCallback(
    async (jobId: string) => {
      try {
        const task = await api.cancelDownload(jobId);
        mergeTask(setTasks, task);
      } catch (error) {
        showNotice("error", "Cancel failed", errorMessage(error));
      }
    },
    [showNotice],
  );

  const resumeInterrupted = useCallback(
    async (task: DownloadTask) => {
      try {
        const resumed = await api.resumeInterrupted(task.id);
        mergeTask(setTasks, resumed);
        setActiveJobId(resumed.id);
        showNotice(
          "success",
          "Resumed",
          `${resumed.title} · unfinished chapters are downloading.`,
        );
      } catch (error) {
        showNotice("error", "Could not resume", errorMessage(error));
      }
    },
    [showNotice],
  );

  const confirmQuit = useCallback(async () => {
    try {
      await api.confirmQuit();
    } catch (error) {
      showNotice("error", "Could not close NovelForge", errorMessage(error));
    }
  }, [showNotice]);

  const dismissQuit = useCallback(() => setQuitRequested(false), []);

  const clearFinished = useCallback(async () => {
    try {
      const remaining = await api.clearFinished();
      setTasks(remaining);
    } catch (error) {
      showNotice("error", "Could not clear finished tasks", errorMessage(error));
    }
  }, [showNotice]);

  const saveSettings = useCallback(
    async (next: Settings): Promise<boolean> => {
      try {
        const saved = await api.saveSettings(next);
        setSettings(saved);
        showNotice("success", "Settings saved", "The toolchain configuration is up to date.");
        void refreshTools();
        void refreshLibrary();
        return true;
      } catch (error) {
        showNotice("error", "Could not save settings", errorMessage(error));
        return false;
      }
    },
    [refreshLibrary, refreshTools, showNotice],
  );

  const verify = useCallback(
    async (path: string): Promise<VerificationReport | null> => {
      try {
        return await api.verifyPath(path);
      } catch (error) {
        showNotice("error", "Verification failed", errorMessage(error));
        return null;
      }
    },
    [showNotice],
  );

  const reveal = useCallback(
    async (path: string) => {
      try {
        await api.revealPath(path);
      } catch (error) {
        showNotice("error", "Could not open folder", errorMessage(error));
      }
    },
    [showNotice],
  );

  const openChapters = useCallback(
    async (folder: string) => {
      try {
        await api.openChapters(folder);
      } catch (error) {
        showNotice("error", "Could not open chapters", errorMessage(error));
      }
    },
    [showNotice],
  );

  const deleteNovel = useCallback(
    async (folder: string): Promise<boolean> => {
      try {
        await api.deleteDownload(folder);
        await refreshLibrary();
        showNotice("success", "Folder deleted", "The structured download was removed from disk.");
        return true;
      } catch (error) {
        showNotice("error", "Could not delete folder", errorMessage(error));
        return false;
      }
    },
    [refreshLibrary, showNotice],
  );

  const cleanupEngineWork = useCallback(async (): Promise<CleanupReport | null> => {
    try {
      const report = await api.cleanupEngineWork();
      await refreshLibrary();
      showNotice(
        report.errors.length > 0 ? "error" : "success",
        report.removed.length > 0
          ? `Freed ${formatBytes(report.bytesFreed)}`
          : "No abandoned work files",
        report.errors.length > 0
          ? `${report.removed.length} removed · ${report.errors.length} failed`
          : report.kept.length > 0
            ? `${report.removed.length} removed · ${report.kept.length} kept for running jobs`
            : `${report.removed.length} work tree(s) removed.`,
      );
      return report;
    } catch (error) {
      showNotice("error", "Could not clean engine work files", errorMessage(error));
      return null;
    }
  }, [refreshLibrary, showNotice]);

  const deletePartialDownloads = useCallback(async (): Promise<CleanupReport | null> => {
    try {
      const report = await api.deletePartialDownloads();
      await refreshLibrary();
      showNotice(
        report.errors.length > 0 ? "error" : "success",
        report.removed.length > 0
          ? `Deleted ${report.removed.length} partial download(s)`
          : "No partial downloads",
        `${formatBytes(report.bytesFreed)} reclaimed${
          report.kept.length > 0 ? ` · ${report.kept.length} kept for running jobs` : ""
        }${report.errors.length > 0 ? ` · ${report.errors.length} failed` : ""}`,
      );
      return report;
    } catch (error) {
      showNotice("error", "Could not delete partial downloads", errorMessage(error));
      return null;
    }
  }, [refreshLibrary, showNotice]);

  const retryNovel = useCallback(
    async (folder: string, refresh = false): Promise<boolean> => {
      try {
        const task = await api.retryDownload(folder, refresh);
        mergeTask(setTasks, task);
        setActiveJobId(task.id);
        showNotice(
          "success",
          refresh ? "Refresh started" : "Retry started",
          refresh
            ? `${task.title} · every chapter will be downloaded again.`
            : `${task.title} · missing chapters are downloading.`,
        );
        return true;
      } catch (error) {
        showNotice("error", "Could not retry download", errorMessage(error));
        return false;
      }
    },
    [showNotice],
  );

  const updateNovel = useCallback(
    async (folder: string, refresh = false): Promise<boolean> => {
      try {
        const task = await api.updateDownload(folder, refresh);
        mergeTask(setTasks, task);
        setActiveJobId(task.id);
        showNotice(
          "success",
          refresh ? "Refresh started" : "Update started",
          refresh
            ? `${task.title} · every chapter will be downloaded again.`
            : `${task.title} · new chapters are downloading; existing files are preserved.`,
        );
        return true;
      } catch (error) {
        showNotice("error", "Could not update download", errorMessage(error));
        return false;
      }
    },
    [showNotice],
  );

  const findFasterSource = useCallback(
    async (folder: string) => {
      try {
        return await api.suggestFasterSource(folder);
      } catch (error) {
        showNotice(
          "error",
          "Could not check for a faster source",
          errorMessage(error),
        );
        return null;
      }
    },
    [showNotice],
  );

  const startAlternative = useCallback(
    async (title: string, url: string, host: string): Promise<boolean> => {
      try {
        const task = await api.startDownload({
          engine: "webnovel",
          query: url,
          title,
          options: { duplicate: "copy" },
        });
        mergeTask(setTasks, task);
        setActiveJobId(task.id);
        showNotice(
          "success",
          "New download started",
          `${task.title} is downloading from ${host}. The existing library entry is unchanged.`,
        );
        return true;
      } catch (error) {
        showNotice("error", "Could not start the new source", errorMessage(error));
        return false;
      }
    },
    [showNotice],
  );

  const getNovelDetails = useCallback(
    async (folder: string): Promise<DownloadDetails | null> => {
      try {
        return await api.getDownloadDetails(folder);
      } catch (error) {
        showNotice("error", "Could not read chapter index", errorMessage(error));
        return null;
      }
    },
    [showNotice],
  );

  const adoptArchive = useCallback(
    async (path: string): Promise<boolean> => {
      try {
        const novel = await api.adoptArchive(path);
        await refreshLibrary();
        showNotice(
          "success",
          "Archive adopted",
          `${novel.title} · ${novel.downloadedChapters.toLocaleString()} chapters indexed; the original file was left untouched.`,
        );
        return true;
      } catch (error) {
        showNotice("error", "Could not adopt archive", errorMessage(error));
        return false;
      }
    },
    [refreshLibrary, showNotice],
  );

  const openDownloads = useCallback(async () => {
    try {
      await api.openDownloads();
    } catch (error) {
      showNotice("error", "Could not open downloads", errorMessage(error));
    }
  }, [showNotice]);

  const clearLogs = useCallback(() => setLogs([]), []);

  const orderedTasks = useMemo(() => {
    const rank: Record<string, number> = {
      running: 0,
      verifying: 1,
      paused: 2,
      queued: 3,
      failed: 4,
      completed: 5,
      cancelled: 6,
    };
    return [...tasks].sort((left, right) => {
      const delta = (rank[left.status] ?? 9) - (rank[right.status] ?? 9);
      if (delta !== 0) return delta;
      return right.createdAt - left.createdAt;
    });
  }, [tasks]);

  const activeTasks = useMemo(
    () => orderedTasks.filter((task) => isActive(task.status)),
    [orderedTasks],
  );

  const currentTask = useMemo(
    () =>
      orderedTasks.find((task) => task.status === "running") ??
      orderedTasks.find((task) => task.status === "verifying") ??
      orderedTasks.find((task) => task.status === "paused") ??
      null,
    [orderedTasks],
  );

  const overallProgress = useMemo(() => {
    const relevant = tasks.filter(
      (task) => task.status === "running" || task.status === "verifying",
    );
    if (relevant.length === 0) return 0;
    return (
      relevant.reduce((total, task) => total + task.progress, 0) / relevant.length
    );
  }, [tasks]);

  const queue = useMemo(
    () =>
      orderedTasks
        .filter((task) => !isTerminal(task.status))
        .slice(0, 5),
    [orderedTasks],
  );

  const toolsList = useMemo<ToolStatus[]>(() => {
    if (!tools) return [];
    return [tools.lncrawl, tools.galleryDl, tools.aget];
  }, [tools]);

  return {
    tasks: orderedTasks,
    logs,
    tools,
    toolsList,
    settings,
    library,
    appInfo,
    loading,
    starting,
    notice,
    activeJobId,
    activeTasks,
    currentTask,
    overallProgress,
    queue,
    mangaSources,
    sourcesLoading,
    resolving,
    setActiveJobId,
    setNotice,
    quitRequested,
    resumeInterrupted,
    confirmQuit,
    dismissQuit,
    searchNovels,
    resolveTitle,
    loadMangaSources,
    start,
    pause,
    resume,
    cancel,
    clearFinished,
    clearLogs,
    refreshLibrary,
    refreshTools,
    saveSettings,
    verify,
    reveal,
    openChapters,
    deleteNovel,
    cleanupEngineWork,
    deletePartialDownloads,
    retryNovel,
    updateNovel,
    findFasterSource,
    startAlternative,
    getNovelDetails,
    adoptArchive,
    openDownloads,
    showNotice,
  };
}

export type DownloadManager = ReturnType<typeof useDownloadManager>;

function mergeTask(
  setTasks: Dispatch<SetStateAction<DownloadTask[]>>,
  task: DownloadTask,
) {
  setTasks((current) =>
    current.some((existing) => existing.id === task.id)
      ? current.map((existing) => (existing.id === task.id ? task : existing))
      : [...current, task],
  );
}

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return "An unexpected error occurred.";
}
