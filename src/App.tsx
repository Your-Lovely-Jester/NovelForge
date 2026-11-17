import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Page } from "@/types";
import { usePageRoute } from "@/lib/routing";
import { useDownloadManager } from "@/hooks/useDownloadManager";
import { Sidebar } from "@/components/shell/Sidebar";
import { Titlebar } from "@/components/shell/Titlebar";
import { RightRail } from "@/components/shell/RightRail";
import { NoticeToast } from "@/components/ui/NoticeToast";
import { HomePage } from "@/pages/HomePage";
import { WebnovelPage } from "@/pages/WebnovelPage";
import { MangaPage } from "@/pages/MangaPage";
import { BooksPage } from "@/pages/BooksPage";
import { LibraryPage } from "@/pages/LibraryPage";
import { DownloadsPage } from "@/pages/DownloadsPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { GuardrailsPage } from "@/pages/GuardrailsPage";
import { AboutPage } from "@/pages/AboutPage";

const RAIL_PAGES: Page[] = ["home", "webnovels", "manga", "books"];

interface SearchSeed {
  query: string;
  nonce: number;
}

export default function App() {
  const manager = useDownloadManager();
  const [page, setPage] = usePageRoute();
  const [seed, setSeed] = useState<SearchSeed>({ query: "", nonce: 0 });
  const mainRef = useRef<HTMLElement>(null);

  useEffect(() => {
    mainRef.current?.scrollTo({ top: 0 });
  }, [page]);

  const readyCount = manager.toolsList.filter((tool) => tool.found).length;
  const enginesTotal = manager.toolsList.length || 3;
  const showRail = RAIL_PAGES.includes(page);

  const currentTaskId = manager.currentTask?.id ?? null;

  const inspect = useCallback(() => {
    if (currentTaskId) manager.setActiveJobId(currentTaskId);
  }, [currentTaskId, manager]);

  const openNovelSearch = useCallback((query: string) => {
    setSeed((current) => ({ query, nonce: current.nonce + 1 }));
    setPage("webnovels");
  }, []);

  const content = useMemo(() => {
    switch (page) {
      case "home":
        return (
          <HomePage
            manager={manager}
            enginesReady={readyCount}
            enginesTotal={enginesTotal}
            onNavigate={setPage}
            onSearch={openNovelSearch}
          />
        );
      case "webnovels":
        return (
          <WebnovelPage
            manager={manager}
            initialQuery={seed.query}
            searchNonce={seed.nonce}
          />
        );
      case "manga":
        return <MangaPage manager={manager} onOpenSettings={() => setPage("settings")} />;
      case "books":
        return <BooksPage manager={manager} onOpenSettings={() => setPage("settings")} />;
      case "library":
        return <LibraryPage manager={manager} />;
      case "downloads":
        return <DownloadsPage manager={manager} />;
      case "settings":
        return <SettingsPage manager={manager} />;
      case "guardrails":
        return <GuardrailsPage manager={manager} />;
      case "about":
        return <AboutPage manager={manager} />;
      default:
        return null;
    }
  }, [enginesTotal, manager, openNovelSearch, page, readyCount, seed]);

  return (
    <div className="app-ambient relative flex h-screen min-h-0 overflow-hidden bg-[#08090a] text-parchment-300">
      <Sidebar
        page={page}
        onNavigate={setPage}
        activeCount={manager.activeTasks.length}
        libraryCount={manager.library.novels?.length ?? 0}
        libraryBytes={manager.library.stats.totalBytes}
        onOpenDownloads={() => void manager.openDownloads()}
      />

      <div className="relative z-10 flex min-w-0 flex-1 flex-col">
        <Titlebar
          page={page}
          online={readyCount === enginesTotal}
          activeCount={manager.activeTasks.length}
        />

        <div className="flex min-h-0 flex-1">
          <div className="flex min-w-0 flex-1 flex-col">
            <main
              ref={mainRef}
              className="min-h-0 flex-1 overflow-y-auto px-4 pb-10 pt-4 sm:px-6 xl:px-8 xl:pt-6"
            >
              <div className="mx-auto min-h-full w-full min-w-0 max-w-[1680px] animate-fade-in">
                {content}
              </div>
            </main>
          </div>

          {showRail && (
            <RightRail
              percent={manager.overallProgress}
              activeTasks={manager.activeTasks}
              sessionBytes={manager.tasks.reduce(
                (total, task) => total + task.bytes,
                0,
              )}
              currentTask={manager.currentTask}
              queue={manager.queue}
              novels={manager.library.novels ?? []}
              libraryBytes={manager.library.stats.totalBytes}
              onSelectTask={manager.setActiveJobId}
              onInspect={inspect}
              onPause={(id) => void manager.pause(id)}
              onResume={(id) => void manager.resume(id)}
              onCancel={(id) => void manager.cancel(id)}
              onOpenLibrary={() => setPage("library")}
              onOpenFolder={() => void manager.openDownloads()}
            />
          )}
        </div>
      </div>

      {manager.quitRequested && (
        <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/70 px-4 backdrop-blur-sm">
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="quit-title"
            className="panel w-full max-w-md p-6"
          >
            <p className="text-[11px] font-semibold uppercase tracking-[0.2em] text-amber-300">
              Download in progress
            </p>
            <h2
              id="quit-title"
              className="mt-2 font-display text-[24px] text-parchment-50"
            >
              Close NovelForge?
            </h2>
            <p className="mt-2 text-[14px] leading-relaxed text-parchment-400">
              {manager.activeTasks.length} download
              {manager.activeTasks.length === 1 ? " is" : "s are"} still running.
              Closing stops them and marks them interrupted. Partial files stay
              on disk, and you can resume from Downloads after the next launch.
            </p>
            <div className="mt-5 flex flex-wrap justify-end gap-2">
              <button
                type="button"
                className="btn-secondary"
                onClick={manager.dismissQuit}
              >
                Keep downloading
              </button>
              <button
                type="button"
                className="btn-primary"
                onClick={() => void manager.confirmQuit()}
              >
                Stop and quit
              </button>
            </div>
          </div>
        </div>
      )}

      <div className="pointer-events-none fixed bottom-5 right-5 z-50">
        {manager.notice && (
          <NoticeToast
            notice={manager.notice}
            onClose={() => manager.setNotice(null)}
          />
        )}
      </div>
    </div>
  );
}
