import { useMemo, useState } from "react";
import {
  AlertTriangle,
  Archive,
  BookOpen,
  CheckCircle2,
  Copy,
  FileArchive,
  FolderOpen,
  Gauge,
  HardDrive,
  List,
  Loader2,
  RefreshCw,
  RotateCcw,
  Search,
  ShieldCheck,
  Trash2,
  X,
  Zap,
} from "lucide-react";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import { errorMessage } from "@/hooks/useDownloadManager";
import type {
  DownloadDetails,
  FasterSourceReport,
  LibraryArchive,
  LibraryNovel,
  LibraryStorage,
} from "@/types";
import { cn, formatBytes, formatDate, formatPace, hostFromUrl, pluralize } from "@/lib/utils";
import { PageHeader } from "@/components/ui/PageHeader";
import { TitlePlate } from "@/components/ui/TitlePlate";

const KIND_STYLE: Record<string, string> = {
  epub: "border-vermilion-500/30 bg-vermilion-500/10 text-vermilion-200",
  cbz: "border-gold-400/25 bg-gold-400/10 text-gold-300",
  zip: "border-gold-400/25 bg-gold-400/10 text-gold-300",
  pdf: "border-parchment-200/20 bg-parchment-100/[0.05] text-parchment-200",
  mobi: "border-jade-400/25 bg-jade-400/10 text-jade-300",
  azw3: "border-jade-400/25 bg-jade-400/10 text-jade-300",
  txt: "border-parchment-100/12 bg-parchment-100/[0.04] text-parchment-400",
};

const FILTERS = ["all", "epub", "cbz", "pdf"] as const;

export function LibraryPage({ manager }: { manager: DownloadManager }) {
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<(typeof FILTERS)[number]>("all");
  const [refreshing, setRefreshing] = useState(false);
  const [verifying, setVerifying] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<{
    kind: "delete" | "refresh";
    novel: LibraryNovel;
  } | null>(null);
  const [busyFolder, setBusyFolder] = useState<string | null>(null);
  const [indexNovel, setIndexNovel] = useState<LibraryNovel | null>(null);
  const [indexDetails, setIndexDetails] = useState<DownloadDetails | null>(null);
  const [indexLoading, setIndexLoading] = useState(false);
  const [adopting, setAdopting] = useState<string | null>(null);
  const [checkingFaster, setCheckingFaster] = useState<string | null>(null);
  const [faster, setFaster] = useState<{
    novel: LibraryNovel;
    report: FasterSourceReport;
  } | null>(null);
  const [cleanup, setCleanup] = useState<"partials" | "work" | null>(null);
  const [cleaning, setCleaning] = useState(false);

  const novels = manager.library.novels ?? [];
  const archives = manager.library.archives ?? [];
  const storage: LibraryStorage = manager.library.storage ?? {
    libraryBytes: manager.library.stats.totalBytes,
    partialBytes: 0,
    partialFolders: 0,
    workBytes: 0,
    workFolders: 0,
    quotaBytes: 0,
    overQuota: false,
  };
  const completeNovels = novels.filter((novel) => novel.complete).length;
  const migratedNovels = novels.filter(
    (novel) => novel.provenance === "migrated",
  ).length;
  const adoptableArchives = archives.filter((archive) => archive.adoptable).length;

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return manager.library.entries.filter((entry) => {
      const matchesKind =
        filter === "all" ||
        entry.kind === filter ||
        (filter === "cbz" && entry.kind === "zip");
      const matchesSearch =
        needle.length === 0 ||
        entry.name.toLowerCase().includes(needle) ||
        entry.folder.toLowerCase().includes(needle);
      return matchesKind && matchesSearch;
    });
  }, [filter, manager.library.entries, search]);

  const byKind = manager.library.stats.byKind;

  async function refresh() {
    setRefreshing(true);
    await manager.refreshLibrary();
    setRefreshing(false);
  }

  async function verify(path: string, label: string) {
    setVerifying(path);
    const report = await manager.verify(path);
    setVerifying(null);
    if (!report) return;
    if (report.ok) {
      manager.showNotice("success", "Integrity confirmed", `${label} · ${report.detail}`);
    } else {
      manager.showNotice("error", "Verification failed", `${label} · ${report.detail}`);
    }
  }

  async function quickAction(novel: LibraryNovel, action: "retry" | "update") {
    setBusyFolder(novel.folder);
    if (action === "retry") {
      await manager.retryNovel(novel.folder);
    } else {
      await manager.updateNovel(novel.folder);
    }
    setBusyFolder(null);
  }

  async function confirmAction() {
    if (!confirm) return;
    const { kind, novel } = confirm;
    setBusyFolder(novel.folder);
    const ok =
      kind === "delete"
        ? await manager.deleteNovel(novel.folder)
        : novel.complete
          ? await manager.updateNovel(novel.folder, true)
          : await manager.retryNovel(novel.folder, true);
    setBusyFolder(null);
    if (ok) setConfirm(null);
  }

  async function viewIndex(novel: LibraryNovel) {
    setIndexNovel(novel);
    setIndexDetails(null);
    setIndexLoading(true);
    const details = await manager.getNovelDetails(novel.folder);
    setIndexLoading(false);
    if (details) {
      setIndexDetails(details);
    } else {
      setIndexNovel(null);
    }
  }

  async function adopt(archive: LibraryArchive) {
    setAdopting(archive.path);
    await manager.adoptArchive(archive.path);
    setAdopting(null);
  }

  async function checkFasterSource(novel: LibraryNovel) {
    setCheckingFaster(novel.folder);
    const report = await manager.findFasterSource(novel.folder);
    setCheckingFaster(null);
    if (report) setFaster({ novel, report });
  }

  async function startFasterSource() {
    const alternative = faster?.report.alternative;
    if (!faster || !alternative) return;
    const { novel } = faster;
    setBusyFolder(novel.folder);
    const ok = await manager.startAlternative(
      novel.title,
      alternative.url,
      alternative.host,
    );
    setBusyFolder(null);
    if (ok) setFaster(null);
  }

  async function runCleanup() {
    if (!cleanup) return;
    setCleaning(true);
    if (cleanup === "partials") {
      await manager.deletePartialDownloads();
    } else {
      await manager.cleanupEngineWork();
    }
    setCleaning(false);
    setCleanup(null);
  }

  async function copySource(novel: LibraryNovel) {
    if (!novel.sourceUrl) return;
    try {
      await navigator.clipboard.writeText(novel.sourceUrl);
      manager.showNotice("success", "Source URL copied", novel.sourceUrl);
    } catch (error) {
      manager.showNotice("error", "Could not copy source URL", errorMessage(error));
    }
  }

  return (
    <div className="space-y-4 pb-2 pt-0">
      <PageHeader
        eyebrow="Local archive"
        title="Library"
        description="Every novel NovelForge has produced, with its real metadata, complete chapter index and verified container. Audit any output again at any time."
        actions={
          <>
            <button
              type="button"
              onClick={() => void refresh()}
              className="btn-secondary"
              disabled={refreshing}
            >
              <RefreshCw
                className={cn("h-3 w-3", refreshing && "animate-spin")}
                strokeWidth={1.8}
              />
              Rescan
            </button>
            <button
              type="button"
              onClick={() => void manager.openDownloads()}
              className="btn-secondary"
            >
              <FolderOpen className="h-3 w-3" strokeWidth={1.8} />
              Open folder
            </button>
          </>
        }
      />

      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <SummaryTile label="Downloaded novels" value={String(novels.length)} accent />
        <SummaryTile
          label="Adopted archives"
          value={String(migratedNovels)}
        />
        <SummaryTile
          label="Loose archive files"
          value={String(archives.length)}
        />
        <SummaryTile
          label="Library size"
          value={formatBytes(storage.libraryBytes)}
        />
      </div>

      <section className="panel p-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex items-start gap-2">
            <HardDrive
              className="mt-[2px] h-3.5 w-3.5 shrink-0 text-vermilion-300/80"
              strokeWidth={1.8}
            />
            <div>
              <div className="text-[13.5px] font-semibold text-parchment-200">
                Disk usage
              </div>
              <div className="mt-0.5 text-[11.5px] text-parchment-600">
                Measured from the folder tree, not an estimate.
              </div>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-1.5">
            <button
              type="button"
              onClick={() => setCleanup("work")}
              disabled={storage.workFolders === 0 || cleaning}
              className="btn-secondary h-7 px-2.5"
              title="Remove abandoned per-job crawler trees"
            >
              <RefreshCw className="h-3 w-3" strokeWidth={1.8} />
              Clean work files
            </button>
            <button
              type="button"
              onClick={() => setCleanup("partials")}
              disabled={storage.partialFolders === 0 || cleaning}
              className="btn-secondary h-7 px-2.5"
              title="Delete every incomplete structured folder"
            >
              <Trash2 className="h-3 w-3" strokeWidth={1.8} />
              Delete partial downloads
            </button>
          </div>
        </div>

        <div className="mt-3 grid grid-cols-2 gap-3 lg:grid-cols-4">
          <StorageStat
            label="Library"
            value={formatBytes(storage.libraryBytes)}
            detail={
              storage.quotaBytes > 0
                ? `quota ${formatBytes(storage.quotaBytes)}`
                : "no quota"
            }
          />
          <StorageStat
            label="Partial output"
            value={formatBytes(storage.partialBytes)}
            detail={pluralize(storage.partialFolders, "folder")}
          />
          <StorageStat
            label="Engine work files"
            value={formatBytes(storage.workBytes)}
            detail={pluralize(storage.workFolders, "folder")}
          />
          <StorageStat
            label="Free of quota"
            value={
              storage.quotaBytes > 0
                ? formatBytes(
                    Math.max(storage.quotaBytes - storage.libraryBytes, 0),
                  )
                : "—"
            }
            detail={
              storage.quotaBytes > 0 ? "before new downloads stop" : "unlimited"
            }
          />
        </div>

        {storage.overQuota && (
          <p className="mt-3 flex items-start gap-1.5 border border-vermilion-500/30 bg-vermilion-500/[0.06] px-2.5 py-2 text-[11px] leading-relaxed text-vermilion-200">
            <AlertTriangle className="mt-px h-3 w-3 shrink-0" strokeWidth={2} />
            Library quota reached. New downloads are refused until you delete
            partial output, remove a novel, or raise the limit in Settings.
            Existing updates and retries still work.
          </p>
        )}
      </section>

      {novels.length > 0 && (
        <section className="space-y-2.5">
          <div className="flex flex-wrap items-end justify-between gap-3 px-1">
            <div>
              <div className="section-label">Structured downloads</div>
              <h2 className="mt-1 font-display text-[20px] leading-none text-parchment-100">
                Downloaded novels
              </h2>
            </div>
            <div className="flex items-center gap-3 text-[11px] text-parchment-500">
              {completeNovels < novels.length && (
                <span className="flex items-center gap-1.5 text-gold-300">
                  <AlertTriangle className="h-3 w-3" strokeWidth={1.8} />
                  {novels.length - completeNovels} incomplete
                </span>
              )}
              <span className="flex items-center gap-1.5">
                <CheckCircle2 className="h-3 w-3 text-jade-400/80" strokeWidth={1.8} />
                {pluralize(completeNovels, "complete download")}
              </span>
            </div>
          </div>

          <div className="grid gap-3 lg:grid-cols-2">
            {novels.map((novel) => (
              <NovelCard
                key={novel.folder}
                novel={novel}
                verifying={verifying}
                busy={busyFolder === novel.folder}
                onOpen={() => void manager.reveal(novel.folder)}
                onArchive={
                  novel.archivePath
                    ? () => void manager.openFile(novel.archivePath as string)
                    : undefined
                }
                onVerify={() => {
                  const target = novel.epubPath ?? novel.archivePath;
                  if (target) {
                    void verify(target, novel.title);
                  } else {
                    manager.showNotice(
                      "error",
                      "Nothing to verify",
                      "This folder has no bundled EPUB or archive.",
                    );
                  }
                }}
                onRetry={() => void quickAction(novel, "retry")}
                onUpdate={() => void quickAction(novel, "update")}
                onRefresh={() => setConfirm({ kind: "refresh", novel })}
                onChapters={() => void manager.openChapters(novel.folder)}
                onRepack={
                  // A comic folder that lost (or never had) its archive shows
                  // the page tree but no CBZ action; offer to rebuild it.
                  !novel.epubPath && !novel.archivePath
                    ? () => void manager.repackManga(novel.folder)
                    : undefined
                }
                onIndex={() => void viewIndex(novel)}
                onCopySource={() => void copySource(novel)}
                onFaster={() => void checkFasterSource(novel)}
                checkingFaster={checkingFaster === novel.folder}
                onDelete={() => setConfirm({ kind: "delete", novel })}
              />
            ))}
          </div>
        </section>
      )}

      {archives.length > 0 && (
        <section className="panel overflow-hidden">
          <div className="flex flex-wrap items-end justify-between gap-3 border-b border-parchment-100/8 px-4 py-3">
            <div>
              <div className="section-label">Migration queue</div>
              <h2 className="mt-1 font-display text-[20px] leading-none text-parchment-100">
                Unstructured archives
              </h2>
            </div>
            <span className="flex items-center gap-1.5 text-[11px] text-parchment-500">
              <Archive className="h-3 w-3 text-gold-400/80" strokeWidth={1.8} />
              {pluralize(adoptableArchives, "file")} ready to adopt
            </span>
          </div>

          <div>
            {archives.map((archive) => (
              <div
                key={archive.path}
                className="flex flex-wrap items-center gap-3 border-b border-parchment-100/[0.05] px-4 py-3 last:border-0"
              >
                <span
                  className={cn(
                    "flex h-8 w-7 shrink-0 items-center justify-center border font-mono text-[9px] font-semibold uppercase tracking-[0.06em]",
                    KIND_STYLE[archive.kind] ??
                      "border-parchment-100/12 bg-parchment-100/[0.03] text-parchment-400",
                  )}
                >
                  {archive.kind}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13px] font-medium text-parchment-200">
                    {archive.name}
                  </div>
                  <div className="mt-0.5 truncate text-[10.5px] text-parchment-600">
                    {archive.folder} · {formatBytes(archive.size)} ·{" "}
                    {formatDate(archive.modified)}
                  </div>
                  <div
                    className={cn(
                      "mt-1 text-[10.5px] leading-relaxed",
                      archive.adoptable ? "text-jade-300/90" : "text-parchment-500",
                    )}
                  >
                    {archive.reason}
                  </div>
                </div>
                {archive.adoptable ? (
                  <button
                    type="button"
                    onClick={() => void adopt(archive)}
                    disabled={adopting === archive.path}
                    className="btn-primary h-7 px-2.5"
                    title="Create metadata, chapter files and a cover beside the original EPUB"
                  >
                    {adopting === archive.path ? (
                      <Loader2 className="h-3 w-3 animate-spin" strokeWidth={2} />
                    ) : (
                      <Archive className="h-3 w-3" strokeWidth={1.9} />
                    )}
                    Adopt
                  </button>
                ) : (
                  <span className="border border-parchment-100/10 bg-parchment-100/[0.03] px-2 py-1 text-[9.5px] font-semibold uppercase tracking-[0.12em] text-parchment-500">
                    Archive only
                  </span>
                )}
                <button
                  type="button"
                  onClick={() => void manager.reveal(archive.path)}
                  aria-label={`Reveal ${archive.name}`}
                  title="Show in file manager"
                  className="icon-button h-7 w-7"
                >
                  <FolderOpen className="h-3.5 w-3.5" strokeWidth={1.8} />
                </button>
              </div>
            ))}
          </div>
        </section>
      )}

      <section className="panel overflow-hidden">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-parchment-100/8 px-4 py-2.5">
          <div className="flex items-center gap-3">
            <span className="section-label">Archive files</span>
            <div className="flex items-center border border-parchment-100/9">
              {FILTERS.map((value) => (
                <button
                  key={value}
                  type="button"
                  onClick={() => setFilter(value)}
                  className={cn(
                    "h-7 border-r border-parchment-100/8 px-2.5 text-[10px] font-semibold uppercase tracking-[0.12em] transition-colors last:border-r-0",
                    filter === value
                      ? "bg-vermilion-500/12 text-vermilion-200"
                      : "text-parchment-500 hover:bg-parchment-100/[0.03] hover:text-parchment-200",
                  )}
                >
                  {value}
                </button>
              ))}
            </div>
          </div>

          <div className="flex items-center gap-2.5">
            <div className="field h-8 w-[250px] gap-2 px-2.5">
              <Search className="h-3 w-3 text-parchment-600" strokeWidth={1.8} />
              <input
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder="Filter by file or folder…"
                aria-label="Filter library"
                spellCheck={false}
                className="text-[12.5px]"
              />
            </div>
            <span className="font-mono text-[11px] tabular-nums text-parchment-600">
              {pluralize(filtered.length, "file")}
            </span>
          </div>
        </div>

        {filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center px-6 py-14 text-center">
            <FolderOpen className="h-6 w-6 text-parchment-600" strokeWidth={1.4} />
            <div className="mt-3 text-[14px] font-medium text-parchment-200">
              {manager.library.entries.length === 0
                ? "Nothing archived yet"
                : "No files match this filter"}
            </div>
            <p className="mt-1.5 max-w-[340px] text-[12px] leading-relaxed text-parchment-600">
              Completed transfers land here automatically once their container
              passes verification.
            </p>
          </div>
        ) : (
          <div className="max-h-[540px] overflow-auto">
            <div className="sticky top-0 z-10 grid min-w-[560px] grid-cols-[minmax(0,1fr)_58px_84px_100px_64px] items-center gap-3 border-b border-parchment-100/8 bg-[#0d0d0f] px-4 py-2">
              {["File", "Type", "Size", "Added", ""].map((label, index) => (
                <span
                  key={`${label}-${index}`}
                  className="text-[9.5px] font-semibold uppercase tracking-[0.16em] text-parchment-600 last:text-right"
                >
                  {label}
                </span>
              ))}
            </div>

            {filtered.map((entry) => (
              <div
                key={entry.path}
                className="group grid min-w-[560px] grid-cols-[minmax(0,1fr)_58px_84px_100px_64px] items-center gap-3 border-b border-parchment-100/[0.05] px-4 py-2.5 transition-colors last:border-0 hover:bg-parchment-100/[0.022]"
              >
                <div className="flex min-w-0 items-center gap-2.5">
                  <span
                    className={cn(
                      "flex h-8 w-7 shrink-0 items-center justify-center border font-mono text-[9px] font-semibold uppercase tracking-[0.06em]",
                      KIND_STYLE[entry.kind] ??
                        "border-parchment-100/12 bg-parchment-100/[0.03] text-parchment-400",
                    )}
                  >
                    {entry.kind}
                  </span>
                  <div className="min-w-0">
                    <div
                      className="truncate text-[13px] font-medium text-parchment-200"
                      title={entry.path}
                    >
                      {entry.name}
                    </div>
                    <div className="mt-0.5 truncate text-[10.5px] normal-case text-parchment-600">
                      {entry.folder}
                    </div>
                  </div>
                </div>
                <span className="font-mono text-[10.5px] uppercase tracking-[0.1em] text-parchment-500">
                  {entry.kind}
                </span>
                <span className="font-mono text-[11.5px] tabular-nums text-parchment-400">
                  {formatBytes(entry.size)}
                </span>
                <span className="text-[11px] text-parchment-500">
                  {formatDate(entry.modified)}
                </span>
                <div className="flex items-center justify-end gap-0.5">
                  <button
                    type="button"
                    onClick={() => void verify(entry.path, entry.name)}
                    disabled={verifying === entry.path}
                    aria-label={`Verify ${entry.name}`}
                    title="Verify container"
                    className="icon-button h-7 w-7"
                  >
                    {verifying === entry.path ? (
                      <RefreshCw className="h-3 w-3 animate-spin" strokeWidth={2} />
                    ) : (
                      <ShieldCheck className="h-3.5 w-3.5" strokeWidth={1.8} />
                    )}
                  </button>
                  <button
                    type="button"
                    onClick={() => void manager.reveal(entry.path)}
                    aria-label={`Reveal ${entry.name}`}
                    title="Show in file manager"
                    className="icon-button h-7 w-7"
                  >
                    <FolderOpen className="h-3.5 w-3.5" strokeWidth={1.8} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </section>

      <div className="flex items-center gap-2 px-1 text-[11px] text-parchment-600">
        <CheckCircle2 className="h-3 w-3 text-jade-400/80" strokeWidth={1.8} />
        Completed novels are re-verified on demand; incomplete folders stay
        visible here for inspection and are never shown on the Home shelf.
        {byKind.epub ? ` EPUB storage ${formatBytes(byKind.epub)}.` : ""}
      </div>

      {confirm && (
        <ConfirmAction
          kind={confirm.kind}
          novel={confirm.novel}
          busy={busyFolder === confirm.novel.folder}
          onCancel={() => setConfirm(null)}
          onConfirm={() => void confirmAction()}
        />
      )}

      {cleanup && (
        <CleanupDialog
          kind={cleanup}
          storage={storage}
          busy={cleaning}
          onCancel={() => setCleanup(null)}
          onConfirm={() => void runCleanup()}
        />
      )}

      {indexNovel && (
        <IndexDialog
          novel={indexNovel}
          details={indexDetails}
          loading={indexLoading}
          onClose={() => {
            setIndexNovel(null);
            setIndexDetails(null);
          }}
        />
      )}

      {faster && (
        <FasterSourceDialog
          novel={faster.novel}
          report={faster.report}
          busy={busyFolder === faster.novel.folder}
          onClose={() => setFaster(null)}
          onStart={() => void startFasterSource()}
        />
      )}
    </div>
  );
}

function NovelCard({
  novel,
  verifying,
  busy,
  onOpen,
  onArchive,
  onVerify,
  onRetry,
  onUpdate,
  onRefresh,
  onChapters,
  onRepack,
  onIndex,
  onCopySource,
  onFaster,
  checkingFaster,
  onDelete,
}: {
  novel: LibraryNovel;
  verifying: string | null;
  busy: boolean;
  onOpen: () => void;
  /** Present only when the folder carries a packed CBZ/ZIP. */
  onArchive?: () => void;
  onVerify: () => void;
  onRetry: () => void;
  onUpdate: () => void;
  onRefresh: () => void;
  onChapters: () => void;
  /** Present when the folder has pages but no packed archive. */
  onRepack?: () => void;
  onIndex: () => void;
  onCopySource: () => void;
  onFaster: () => void;
  checkingFaster: boolean;
  onDelete: () => void;
}) {
  const migrated = novel.provenance === "migrated";
  const source = novel.sourceUrl
    ? hostFromUrl(novel.sourceUrl)
    : migrated
      ? "local EPUB"
      : "local";
  const current = (novel.epubPath ?? novel.archivePath) === verifying;
  const progress =
    novel.chapterCount > 0
      ? Math.min(100, (novel.downloadedChapters / novel.chapterCount) * 100)
      : 0;

  return (
    <article className="panel flex gap-3.5 p-3.5">
      {novel.cover ? (
        <img
          src={novel.cover}
          alt=""
          className="h-[116px] w-[84px] shrink-0 rounded-[4px] object-cover"
          loading="lazy"
        />
      ) : (
        <TitlePlate
          title={novel.title}
          source={novel.folder}
          className="h-[116px] w-[84px] rounded-[4px]"
        />
      )}

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1">
            <h3 className="truncate font-display text-[19px] leading-tight text-parchment-100">
              {novel.title}
            </h3>
            <div className="mt-1 flex min-w-0 items-center gap-1.5 text-[11px] text-parchment-500">
              {novel.author && <span className="truncate">{novel.author}</span>}
              {novel.author && <span className="text-parchment-700">·</span>}
              <span className="truncate">{source}</span>
            </div>
            {novel.crawl && (
              <div
                className="mt-1 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[10.5px] text-parchment-600"
                title={`Measured on ${formatDate(novel.crawl.measuredAt)} from ${novel.crawl.chapters.toLocaleString()} chapter(s) in ${Math.round(novel.crawl.seconds).toLocaleString()} s`}
              >
                <Gauge className="h-2.5 w-2.5 shrink-0 text-jade-400/70" strokeWidth={1.9} />
                <span className="font-mono tabular-nums text-parchment-400">
                  Measured {formatPace(novel.crawl.chaptersPerMinute)}
                </span>
                <span className="text-parchment-700">·</span>
                <span className="font-mono tabular-nums">
                  {novel.crawl.secondsPerChapter.toFixed(1)} s/chapter
                </span>
              </div>
            )}
          </div>
          <div className="flex shrink-0 flex-col items-end gap-1">
            <span
              className={cn(
                "flex items-center gap-1 border px-1.5 py-0.5 text-[9.5px] font-semibold uppercase tracking-[0.12em]",
                novel.complete
                  ? "border-jade-400/25 bg-jade-400/10 text-jade-300"
                  : "border-gold-400/25 bg-gold-400/10 text-gold-300",
              )}
            >
              {novel.complete ? (
                <CheckCircle2 className="h-2.5 w-2.5" strokeWidth={2} />
              ) : (
                <AlertTriangle className="h-2.5 w-2.5" strokeWidth={2} />
              )}
              {novel.complete ? "Complete" : "Incomplete"}
            </span>
            {migrated && (
              <span
                className="flex items-center gap-1 border border-gold-400/25 bg-gold-400/10 px-1.5 py-0.5 text-[9.5px] font-semibold uppercase tracking-[0.12em] text-gold-300"
                title="Adopted from a loose EPUB; the original file was not changed"
              >
                <Archive className="h-2.5 w-2.5" strokeWidth={2} />
                Migrated
              </span>
            )}
          </div>
        </div>

        {novel.tags.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-x-2.5 gap-y-1 text-[11px] text-parchment-500">
            {novel.tags.slice(0, 4).map((tag) => (
              <span key={tag}>{tag}</span>
            ))}
          </div>
        )}

        <p className="mt-2 line-clamp-2 text-[12px] leading-relaxed text-parchment-500">
          {novel.synopsis?.trim() ||
            `${novel.downloadedChapters.toLocaleString()} of ${novel.chapterCount.toLocaleString()} chapters downloaded.`}
        </p>

        {migrated && (
          <p className="mt-1.5 text-[10.5px] leading-relaxed text-gold-300/70">
            Adopted from a loose EPUB; the original file was left untouched.
          </p>
        )}

        <div className="mt-auto pt-2.5">
          <div className="flex items-center justify-between gap-3 text-[10.5px] text-parchment-500">
            <span className="flex items-center gap-1.5">
              <BookOpen className="h-3 w-3" strokeWidth={1.8} />
              {novel.downloadedChapters.toLocaleString()} /{" "}
              {novel.chapterCount.toLocaleString()} chapters
            </span>
            <span>
              {formatBytes(novel.diskBytes)} · {formatDate(novel.modified)}
            </span>
          </div>
          <div className="mt-1.5 h-[3px] overflow-hidden rounded-full bg-parchment-100/[0.06]">
            <div
              className={cn(
                "h-full rounded-full",
                novel.complete ? "bg-jade-400/70" : "bg-gold-400/60",
              )}
              style={{ width: `${Math.max(progress, progress > 0 ? 1.5 : 0)}%` }}
            />
          </div>

          {!novel.complete && novel.retryable && (
            <p className="mt-2 flex items-start gap-1.5 text-[10.5px] leading-relaxed text-gold-300/90">
              <AlertTriangle className="mt-px h-2.5 w-2.5 shrink-0" strokeWidth={2} />
              Partial output is kept for inspection. Retry fetches only the
              chapters whose files are missing.
            </p>
          )}

          <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
            {novel.retryable && !novel.complete && (
              <button
                type="button"
                onClick={onRetry}
                disabled={busy}
                className="btn-secondary h-7 px-2.5"
                title="Download only the missing chapters"
              >
                {busy ? (
                  <Loader2 className="h-3 w-3 animate-spin" strokeWidth={2} />
                ) : (
                  <RotateCcw className="h-3 w-3" strokeWidth={1.9} />
                )}
                Retry missing
              </button>
            )}
            {novel.retryable && novel.complete && (
              <button
                type="button"
                onClick={onUpdate}
                disabled={busy}
                className="btn-secondary h-7 px-2.5"
                title="Fetch new chapters and keep existing files"
              >
                {busy ? (
                  <Loader2 className="h-3 w-3 animate-spin" strokeWidth={2} />
                ) : (
                  <RefreshCw className="h-3 w-3" strokeWidth={1.9} />
                )}
                Update
              </button>
            )}
            {novel.retryable && novel.sourceRank > 0 && (
              <button
                type="button"
                onClick={onFaster}
                disabled={busy || checkingFaster}
                className="btn-secondary h-7 px-2.5"
                title="Check whether a measured-faster host carries this title"
              >
                {checkingFaster ? (
                  <Loader2 className="h-3 w-3 animate-spin" strokeWidth={2} />
                ) : (
                  <Zap className="h-3 w-3" strokeWidth={1.9} />
                )}
                Faster source
              </button>
            )}
            {onArchive && (
              <button
                type="button"
                onClick={onArchive}
                className="btn-secondary h-7 px-2.5"
                title="Open the packed CBZ/ZIP in your comic reader"
              >
                <FileArchive className="h-3 w-3" strokeWidth={1.8} />
                Open CBZ
              </button>
            )}
            {onRepack && (
              <button
                type="button"
                onClick={onRepack}
                disabled={busy}
                className="btn-secondary h-7 px-2.5"
                title="Rebuild the packed CBZ from the page folders"
              >
                <Archive className="h-3 w-3" strokeWidth={1.8} />
                Make CBZ
              </button>
            )}
            <button
              type="button"
              onClick={onChapters}
              className="btn-secondary h-7 px-2.5"
              title="Open the chapter folder"
            >
              <BookOpen className="h-3 w-3" strokeWidth={1.8} />
              Chapters
            </button>
            <button
              type="button"
              onClick={onIndex}
              className="btn-secondary h-7 px-2.5"
              title="Inspect the full chapter index and download state"
            >
              <List className="h-3 w-3" strokeWidth={1.8} />
              Index
            </button>
            <button type="button" onClick={onOpen} className="btn-secondary h-7 px-2.5">
              <FolderOpen className="h-3 w-3" strokeWidth={1.8} />
              Reveal
            </button>
            <button
              type="button"
              onClick={onVerify}
              disabled={current}
              className="btn-secondary h-7 px-2.5"
            >
              {current ? (
                <RefreshCw className="h-3 w-3 animate-spin" strokeWidth={2} />
              ) : (
                <ShieldCheck className="h-3 w-3" strokeWidth={1.8} />
              )}
              Verify EPUB
            </button>
          </div>

          <div className="mt-1.5 flex items-center justify-end gap-0.5">
            {novel.sourceUrl && (
              <button
                type="button"
                onClick={onCopySource}
                aria-label={`Copy source URL for ${novel.title}`}
                title="Copy source URL"
                className="icon-button h-7 w-7"
              >
                <Copy className="h-3 w-3" strokeWidth={1.8} />
              </button>
            )}
            {novel.retryable && (
              <button
                type="button"
                onClick={onRefresh}
                disabled={busy}
                aria-label={`Refresh every chapter of ${novel.title}`}
                title="Refresh every chapter"
                className="icon-button h-7 w-7"
              >
                <RefreshCw className="h-3 w-3" strokeWidth={1.8} />
              </button>
            )}
            <button
              type="button"
              onClick={onDelete}
              disabled={busy}
              aria-label={`Delete ${novel.title}`}
              title="Delete this download"
              className="icon-button h-7 w-7 hover:text-vermilion-300"
            >
              <Trash2 className="h-3 w-3" strokeWidth={1.8} />
            </button>
          </div>
        </div>
      </div>
    </article>
  );
}

function ConfirmAction({
  kind,
  novel,
  busy,
  onCancel,
  onConfirm,
}: {
  kind: "delete" | "refresh";
  novel: LibraryNovel;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const deleting = kind === "delete";
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="library-confirm-title"
    >
      <div className="w-full max-w-[480px] border border-parchment-100/14 bg-[#0c0c0e] p-5 shadow-2xl">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div
              className={cn(
                "text-[9.5px] font-semibold uppercase tracking-[0.18em]",
                deleting ? "text-vermilion-300" : "text-gold-300",
              )}
            >
              {deleting ? "Destructive action" : "Re-download everything"}
            </div>
            <h2
              id="library-confirm-title"
              className="mt-1 font-display text-[21px] leading-tight text-parchment-50"
            >
              {deleting ? "Delete this download?" : "Refresh every chapter?"}
            </h2>
          </div>
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            aria-label="Cancel"
            className="text-parchment-500 transition-colors hover:text-parchment-200 disabled:text-parchment-700"
          >
            <X className="h-4 w-4" strokeWidth={1.8} />
          </button>
        </div>

        <p className="mt-2 text-[12px] leading-relaxed text-parchment-400">
          {deleting ? (
            <>
              This permanently removes{" "}
              <span className="text-parchment-200">{novel.title}</span> from disk,
              including its cover, metadata, every chapter file and the bundled
              EPUB. This cannot be undone.
            </>
          ) : (
            <>
              The source index is fetched again and{" "}
              <span className="text-parchment-200">
                all {novel.chapterCount.toLocaleString()} chapters
              </span>{" "}
              are downloaded again. Existing chapter files are replaced as they
              are rewritten.
            </>
          )}
        </p>

        <div className="mt-3 border border-parchment-100/10 bg-black/25 px-3 py-2">
          <div
            className="truncate font-mono text-[10px] text-parchment-500"
            title={novel.folder}
          >
            {novel.folder}
          </div>
          <div className="mt-1.5 flex flex-wrap gap-x-3 gap-y-1 text-[10px] uppercase tracking-[0.1em]">
            <span className="font-mono tabular-nums text-parchment-400">
              {novel.downloadedChapters.toLocaleString()} /{" "}
              {novel.chapterCount.toLocaleString()} chapters
            </span>
            <span className={novel.complete ? "text-jade-300" : "text-gold-300"}>
              {novel.complete ? "complete" : "partial"}
            </span>
          </div>
        </div>

        <div className="mt-4 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="btn-secondary"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={busy}
            className="btn-primary"
          >
            {busy ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
            ) : deleting ? (
              <Trash2 className="h-3.5 w-3.5" strokeWidth={1.9} />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" strokeWidth={1.9} />
            )}
            {deleting ? "Delete permanently" : "Refresh all chapters"}
          </button>
        </div>
      </div>
    </div>
  );
}

function IndexDialog({
  novel,
  details,
  loading,
  onClose,
}: {
  novel: LibraryNovel;
  details: DownloadDetails | null;
  loading: boolean;
  onClose: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="library-index-title"
    >
      <div className="flex max-h-[85vh] w-full max-w-[680px] flex-col border border-parchment-100/14 bg-[#0c0c0e] shadow-2xl">
        <div className="flex items-start justify-between gap-4 border-b border-parchment-100/10 p-5">
          <div className="min-w-0">
            <div className="text-[9.5px] font-semibold uppercase tracking-[0.18em] text-vermilion-300">
              Stored chapter index
            </div>
            <h2
              id="library-index-title"
              className="mt-1 truncate font-display text-[21px] leading-tight text-parchment-50"
            >
              {novel.title}
            </h2>
            <div
              className="mt-1 truncate font-mono text-[10px] text-parchment-600"
              title={details?.sourceUrl ?? novel.sourceUrl ?? novel.folder}
            >
              {details?.sourceUrl ?? novel.sourceUrl ?? novel.folder}
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close index"
            className="text-parchment-500 transition-colors hover:text-parchment-200"
          >
            <X className="h-4 w-4" strokeWidth={1.8} />
          </button>
        </div>

        {loading ? (
          <div className="flex items-center justify-center gap-2 px-6 py-16 text-[12px] text-parchment-500">
            <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
            Reading metadata.json…
          </div>
        ) : !details ? (
          <div className="px-6 py-16 text-center text-[12px] text-parchment-500">
            The stored index could not be read.
          </div>
        ) : (
          <>
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 border-b border-parchment-100/8 px-5 py-2.5 text-[10.5px] uppercase tracking-[0.1em]">
              <span className="font-mono tabular-nums text-parchment-300">
                {details.downloadedChapters.toLocaleString()} /{" "}
                {details.chapterCount.toLocaleString()} chapters
              </span>
              {details.selection && (
                <span className="font-mono text-parchment-500">
                  selection {details.selection}
                </span>
              )}
              <span className={details.complete ? "text-jade-300" : "text-gold-300"}>
                {details.complete ? "complete download" : "partial download"}
              </span>
              <span
                className={
                  details.indexComplete
                    ? "text-jade-300"
                    : novel.provenance === "migrated"
                      ? "text-gold-300"
                      : "text-vermilion-300"
                }
              >
                {details.indexComplete
                  ? "full index"
                  : novel.provenance === "migrated"
                    ? "local EPUB index"
                    : "incomplete index"}
              </span>
            </div>

            {!details.indexComplete && novel.provenance !== "migrated" && (
              <div className="border-b border-vermilion-500/20 bg-vermilion-500/[0.06] px-5 py-2.5 text-[11px] leading-relaxed text-vermilion-200">
                The stored index does not prove it covers the whole source.
                Retry and update are refused until a complete index is fetched.
                {details.indexDetail ? ` ${details.indexDetail}` : ""}
              </div>
            )}

            {novel.provenance === "migrated" && (
              <div className="border-b border-gold-400/20 bg-gold-400/[0.05] px-5 py-2.5 text-[11px] leading-relaxed text-gold-200">
                Adopted from a local EPUB. Chapter order follows the EPUB spine,
                so no remote source index exists and retry/update stay disabled.
                {details.indexDetail ? ` ${details.indexDetail}` : ""}
              </div>
            )}

            <div className="min-h-0 flex-1 overflow-y-auto">
              {details.chapters.length === 0 ? (
                <div className="px-5 py-10 text-center text-[12px] text-parchment-500">
                  metadata.json contains no chapters.
                </div>
              ) : (
                details.chapters.map((chapter) => (
                  <div
                    key={`${chapter.number}-${chapter.title}`}
                    className="flex items-center gap-3 border-b border-parchment-100/[0.05] px-5 py-1.5 text-[11.5px] last:border-0"
                  >
                    <span className="w-10 shrink-0 text-right font-mono text-[10.5px] tabular-nums text-parchment-600">
                      {chapter.number}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-parchment-300">
                      {chapter.title}
                    </span>
                    <span
                      className={cn(
                        "shrink-0 text-[9.5px] font-semibold uppercase tracking-[0.1em]",
                        chapter.downloaded && chapter.file
                          ? "text-jade-400"
                          : "text-gold-300",
                      )}
                    >
                      {chapter.downloaded && chapter.file ? "file" : "missing"}
                    </span>
                  </div>
                ))
              )}
            </div>

            <div className="flex items-center justify-between gap-3 border-t border-parchment-100/10 px-5 py-3">
              <span className="truncate font-mono text-[10px] text-parchment-600" title={details.folder}>
                {details.folder}
              </span>
              <button type="button" onClick={onClose} className="btn-secondary">
                Close
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function SummaryTile({
  label,
  value,
  accent = false,
}: {
  label: string;
  value: string;
  accent?: boolean;
}) {
  return (
    <div className="panel px-4 py-3">
      <div className="section-label">{label}</div>
      <div
        className={cn(
          "mt-1.5 font-display text-[24px] font-semibold leading-none tabular-nums",
          accent ? "text-vermilion-200" : "text-parchment-100",
        )}
      >
        {value}
      </div>
    </div>
  );
}

function StorageStat({
  label,
  value,
  detail,
}: {
  label: string;
  value: string;
  detail: string;
}) {
  return (
    <div className="border border-parchment-100/[0.06] bg-parchment-100/[0.015] px-3 py-2">
      <div className="text-[9.5px] font-semibold uppercase tracking-[0.14em] text-parchment-600">
        {label}
      </div>
      <div className="mt-1 font-mono text-[15px] tabular-nums text-parchment-200">
        {value}
      </div>
      <div className="mt-0.5 text-[10.5px] text-parchment-600">{detail}</div>
    </div>
  );
}

function CleanupDialog({
  kind,
  storage,
  busy,
  onCancel,
  onConfirm,
}: {
  kind: "partials" | "work";
  storage: LibraryStorage;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const partials = kind === "partials";
  const count = partials ? storage.partialFolders : storage.workFolders;
  const bytes = partials ? storage.partialBytes : storage.workBytes;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="library-cleanup-title"
    >
      <div className="w-full max-w-[480px] border border-parchment-100/14 bg-[#0c0c0e] p-5 shadow-2xl">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="text-[9.5px] font-semibold uppercase tracking-[0.18em] text-vermilion-300">
              Destructive action
            </div>
            <h2
              id="library-cleanup-title"
              className="mt-1 font-display text-[21px] leading-tight text-parchment-50"
            >
              {partials
                ? `Delete ${pluralize(count, "partial download")}?`
                : "Clean engine work files?"}
            </h2>
          </div>
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            aria-label="Cancel"
            className="text-parchment-500 transition-colors hover:text-parchment-200 disabled:text-parchment-700"
          >
            <X className="h-4 w-4" strokeWidth={1.8} />
          </button>
        </div>

        <p className="mt-2 text-[12px] leading-relaxed text-parchment-400">
          {partials ? (
            <>
              Removes {pluralize(count, "incomplete folder")} totalling{" "}
              <span className="text-parchment-200">{formatBytes(bytes)}</span>.
              Complete downloads are never included, and a folder a job is still
              writing is kept. This cannot be undone.
            </>
          ) : (
            <>
              Removes abandoned per-job crawler trees under{" "}
              <span className="font-mono text-parchment-200">.novelforge-work</span>{" "}
              totalling {formatBytes(bytes)}. Queued, running and paused jobs are
              kept. This cannot be undone.
            </>
          )}
        </p>

        <div className="mt-4 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="btn-secondary"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={busy || count === 0}
            className="btn-primary"
          >
            {busy ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
            ) : (
              <Trash2 className="h-3.5 w-3.5" strokeWidth={1.9} />
            )}
            {partials ? "Delete partial downloads" : "Clean work files"}
          </button>
        </div>
      </div>
    </div>
  );
}

function FasterSourceDialog({
  novel,
  report,
  busy,
  onClose,
  onStart,
}: {
  novel: LibraryNovel;
  report: FasterSourceReport;
  busy: boolean;
  onClose: () => void;
  onStart: () => void;
}) {
  const alternative = report.alternative;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="library-faster-title"
    >
      <div className="w-full max-w-[520px] border border-parchment-100/14 bg-[#0c0c0e] p-5 shadow-2xl">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5 text-[9.5px] font-semibold uppercase tracking-[0.18em] text-gold-300">
              <Zap className="h-3 w-3" strokeWidth={2} />
              Measured host comparison
            </div>
            <h2
              id="library-faster-title"
              className="mt-1 truncate font-display text-[21px] leading-tight text-parchment-50"
            >
              {novel.title}
            </h2>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            aria-label="Close"
            className="text-parchment-500 transition-colors hover:text-parchment-200 disabled:text-parchment-700"
          >
            <X className="h-4 w-4" strokeWidth={1.8} />
          </button>
        </div>

        <div className="mt-3 space-y-2 border border-parchment-100/10 bg-black/25 p-3">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="section-label">Stored source</div>
              <div
                className="mt-0.5 truncate font-mono text-[11px] text-parchment-300"
                title={novel.sourceUrl ?? undefined}
              >
                {report.currentHost ?? hostFromUrl(novel.sourceUrl ?? "")}
              </div>
            </div>
            <div className="shrink-0 text-right">
              <div className="text-[9.5px] uppercase tracking-[0.12em] text-parchment-600">
                Last run
              </div>
              <div className="mt-0.5 font-mono text-[10.5px] tabular-nums text-parchment-400">
                {novel.crawl
                  ? formatPace(novel.crawl.chaptersPerMinute)
                  : "no pace measured"}
              </div>
            </div>
          </div>

          {alternative && (
            <div className="flex items-start justify-between gap-3 border-t border-parchment-100/8 pt-2">
              <div className="min-w-0">
                <div className="text-[9.5px] font-semibold uppercase tracking-[0.12em] text-jade-300/90">
                  Measured cheaper
                </div>
                <div
                  className="mt-0.5 truncate font-mono text-[11px] text-jade-200"
                  title={alternative.url}
                >
                  {alternative.host}
                </div>
                <div className="mt-0.5 truncate text-[10.5px] text-parchment-500">
                  {alternative.title} · {alternative.detail}
                </div>
              </div>
              <div className="shrink-0 text-right">
                <div className="text-[9.5px] uppercase tracking-[0.12em] text-parchment-600">
                  Match
                </div>
                <div className="mt-0.5 font-mono text-[10.5px] tabular-nums text-jade-300">
                  {Math.round(alternative.confidence * 100)}%
                </div>
              </div>
            </div>
          )}
        </div>

        <p className="mt-3 text-[12px] leading-relaxed text-parchment-400">
          {report.note}
        </p>
        <p className="mt-1.5 text-[10.5px] leading-relaxed text-parchment-600">
          Nothing switches automatically. The existing folder keeps its stored
          source URL and every file already downloaded.
        </p>

        <div className="mt-4 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="btn-secondary"
          >
            {alternative ? "Keep current" : "Close"}
          </button>
          {alternative && (
            <button
              type="button"
              onClick={onStart}
              disabled={busy}
              className="btn-primary"
              title={`Start a separate download from ${alternative.host}`}
            >
              {busy ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
              ) : (
                <Zap className="h-3.5 w-3.5" strokeWidth={1.9} />
              )}
              Download from {alternative.host}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
