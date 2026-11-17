import {
  Archive,
  BookMarked,
  BookOpenText,
  CircuitBoard,
  Compass,
  Cpu,
  ExternalLink,
  FolderOpen,
  Images,
  Layers,
  Network,
  Scale,
  ShieldCheck,
} from "lucide-react";
import { useState } from "react";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import { api } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";
import { PageHeader } from "@/components/ui/PageHeader";
import { ForgeMark } from "@/components/ui/ForgeMark";

const ENGINES = [
  {
    icon: BookOpenText,
    name: "lncrawl",
    label: "Webnovel",
    upstream: "lncrawl/lightnovel-crawler",
    detail: "Crawls NovelFire, NovelPhoenix, NovelFull, FreeWebNovel, Divine Dao Library, NovelPing and the wider lncrawl catalogue; honours chapter selections and builds a single EPUB.",
    tone: "text-vermilion-300",
    ring: "border-vermilion-500/25 bg-vermilion-500/[0.07]",
  },
  {
    icon: Images,
    name: "gallery-dl",
    label: "Manga",
    upstream: "mikf/gallery-dl",
    detail: "Pulls chapters from gallery-dl's live manga extractor catalogue and packs one CBZ or ZIP with chapter folders.",
    tone: "text-gold-300",
    ring: "border-gold-400/20 bg-gold-400/[0.06]",
  },
  {
    icon: BookMarked,
    name: "aget",
    label: "Books",
    upstream: "ALBEDO-TABAI/annas-archive-downloader",
    detail: "Searches WeLib, Anna’s Archive and OceanOfPDF, falling back when a mirror blocks.",
    tone: "text-jade-300",
    ring: "border-jade-400/20 bg-jade-400/[0.06]",
  },
] as const;

const STACK = [
  "Tauri 2",
  "React 19",
  "TypeScript",
  "Tailwind CSS 4",
  "Rust",
  "Tokio",
] as const;

const PIPELINE = [
  {
    icon: Compass,
    title: "Resolve",
    detail:
      "Reads source author pages, scores exact titles, aliases and authors, and caches search rows, author pages and detail summaries so a repeat lookup is a disk read, not another crawler run.",
  },
  {
    icon: Network,
    title: "Fetch",
    detail:
      "Runs each transfer in an isolated work tree, applies per-host concurrency and pacing, honours Retry-After on 429/503, and falls back to curl when a TLS fingerprint is blocked.",
  },
  {
    icon: ShieldCheck,
    title: "Verify",
    detail:
      "Fails closed unless the chapter index is complete, every selected chapter is non-empty, and the finished container passes a real EPUB/OCF or CBZ audit.",
  },
  {
    icon: Archive,
    title: "Archive",
    detail:
      "Writes cover, metadata.json, index.json, chapters.html, chapter XHTML and one container. Chapters are structured as they arrive, so a long crawl is readable and crash-safe before the export finishes; partial output is kept on failure.",
  },
] as const;

const ENGINEERING = [
  "Rust + Tokio job engine with process isolation",
  "Declarative source registry with verified adapters and parsers",
  "TTL caches for search, author pages, summaries and indexes",
  "Fail-closed index and container verification before import",
  "Tauri shell: Linux and Windows installed and launched; macOS awaits runtime validation",
] as const;

export function AboutPage({ manager }: { manager: DownloadManager }) {
  const info = manager.appInfo;
  const [diagState, setDiagState] = useState<"idle" | "copied" | "error">("idle");

  const copyDiagnostics = async () => {
    try {
      const report = await api.getDiagnostics();
      await navigator.clipboard.writeText(report);
      setDiagState("copied");
    } catch {
      setDiagState("error");
    }
    window.setTimeout(() => setDiagState("idle"), 3200);
  };

  return (
    <div className="space-y-4">
      <PageHeader
        eyebrow="The studio"
        title="About NovelForge"
        description="A Rust-backed download studio with its own resolver, transport policy, verification pipeline and structured library — no account, no cloud, no telemetry."
        actions={
          <button
            type="button"
            onClick={() => void manager.openDownloads()}
            className="btn-secondary"
          >
            <FolderOpen className="h-3 w-3" strokeWidth={1.8} />
            Open downloads
          </button>
        }
      />

      <section className="panel hero-surface noise-overlay relative overflow-hidden p-6">
        <span className="vertical-text absolute right-5 top-5 hidden select-none font-display text-[13px] tracking-[0.36em] text-parchment-100/15 sm:block">
          書庫
        </span>
        <div className="relative flex flex-col gap-5 sm:flex-row sm:items-center">
          <div className="relative flex h-16 w-16 shrink-0 items-center justify-center border border-parchment-100/12 bg-black/35 shadow-[inset_0_1px_0_rgba(232,227,216,0.05)]">
            <ForgeMark className="h-9 w-9 text-vermilion-400" />
            <span className="absolute -right-1 -top-1 h-2 w-2 bg-vermilion-500" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="font-display text-[31px] font-semibold leading-none text-parchment-50">
                NovelForge
              </h2>
              <span className="chip chip-vermilion">v{info?.version ?? "0.1.0"}</span>
              <span className="chip">build {info?.commit ?? "dev"}</span>
              <span className="chip">Local-first</span>
            </div>
            <p className="mt-2 max-w-[620px] text-[13px] leading-relaxed text-parchment-400">
              NovelForge is a self-contained desktop pipeline, not a skin over a
              command line. The Rust/Tokio backend resolves titles and authors,
              drives maintained download engines, applies per-host transport
              policy, proves each chapter index complete, audits every EPUB or
              CBZ, and writes a structured library you own.
            </p>
            <div className="mt-3 flex flex-wrap gap-1.5">
              {STACK.map((item) => (
                <span key={item} className="chip">
                  {item}
                </span>
              ))}
            </div>
          </div>
        </div>
      </section>

      <section className="panel p-4 sm:p-5">
        <div className="flex items-center gap-2">
          <Cpu className="h-3.5 w-3.5 text-vermilion-300/80" strokeWidth={1.8} />
          <span className="text-[13px] font-semibold text-parchment-200">
            The pipeline, end to end
          </span>
          <span className="ml-auto hidden font-mono text-[11px] uppercase tracking-[0.14em] text-parchment-600 sm:block">
            resolve → fetch → verify → archive
          </span>
        </div>
        <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
          {PIPELINE.map((step, index) => {
            const Icon = step.icon;
            return (
              <div
                key={step.title}
                className="relative border border-parchment-100/[0.055] bg-parchment-100/[0.015] p-3"
              >
                <div className="flex items-center gap-2">
                  <span className="flex h-7 w-7 items-center justify-center border border-vermilion-500/25 bg-vermilion-500/[0.07]">
                    <Icon className="h-3.5 w-3.5 text-vermilion-300" strokeWidth={1.7} />
                  </span>
                  <span className="text-[13px] font-semibold text-parchment-200">
                    {step.title}
                  </span>
                  <span className="ml-auto font-mono text-[11px] tabular-nums text-parchment-600">
                    0{index + 1}
                  </span>
                </div>
                <p className="mt-2 text-[11.5px] leading-relaxed text-parchment-500">
                  {step.detail}
                </p>
              </div>
            );
          })}
        </div>
      </section>

      <div className="grid grid-cols-1 gap-3.5 xl:grid-cols-12">
        <section className="panel p-4 xl:col-span-7">
          <div className="flex items-center gap-2">
            <Layers className="h-3.5 w-3.5 text-vermilion-300/80" strokeWidth={1.8} />
            <span className="text-[13px] font-semibold text-parchment-200">
              Download engines
            </span>
          </div>
          <div className="mt-3 space-y-2">
            {ENGINES.map((engine) => {
              const Icon = engine.icon;
              return (
                <div
                  key={engine.name}
                  className="flex items-start gap-3 border border-parchment-100/[0.055] bg-parchment-100/[0.015] p-3"
                >
                  <div
                    className={`flex h-9 w-9 shrink-0 items-center justify-center border ${engine.ring}`}
                  >
                    <Icon className={`h-4 w-4 ${engine.tone}`} strokeWidth={1.7} />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-[13px] font-semibold text-parchment-200">
                        {engine.name}
                      </span>
                      <span className="chip h-[20px] px-1.5 text-[10px]">
                        {engine.label}
                      </span>
                    </div>
                    <p className="mt-1 text-[11.5px] leading-relaxed text-parchment-500">
                      {engine.detail}
                    </p>
                    <div className="mt-1 flex items-center gap-1 font-mono text-[10.5px] text-parchment-600">
                      <ExternalLink className="h-2.5 w-2.5" strokeWidth={1.8} />
                      {engine.upstream}
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </section>

        <div className="space-y-3.5 xl:col-span-5">
          <section className="panel p-4">
            <div className="flex items-center gap-2">
              <Cpu className="h-3.5 w-3.5 text-vermilion-300/80" strokeWidth={1.8} />
              <span className="text-[13px] font-semibold text-parchment-200">Runtime</span>
            </div>
            <dl className="mt-3 space-y-2">
              <RuntimeRow label="Platform" value={info?.platform ?? "linux"} />
              <RuntimeRow label="Architecture" value={info?.arch ?? "—"} />
              <RuntimeRow label="Tauri" value={info?.tauriVersion ?? "2"} />
              <RuntimeRow
                label="Library"
                value={`${manager.library.stats.count} files · ${formatBytes(
                  manager.library.stats.totalBytes,
                )}`}
              />
              <div className="border-t border-parchment-100/8 pt-2">
                <dt className="text-[10px] font-semibold uppercase tracking-[0.15em] text-parchment-600">
                  Download root
                </dt>
                <dd className="mt-1 break-all font-mono text-[11px] leading-relaxed text-parchment-400">
                  {info?.downloadDir ?? manager.settings?.downloadDir ?? "—"}
                </dd>
              </div>
            </dl>
            <button
              type="button"
              onClick={() => void copyDiagnostics()}
              className="btn-secondary mt-3 h-8 w-full justify-center text-[12px]"
            >
              {diagState === "copied"
                ? "Diagnostics copied"
                : diagState === "error"
                  ? "Could not copy diagnostics"
                  : "Copy diagnostics"}
            </button>
            <p className="mt-1.5 text-[10.5px] leading-relaxed text-parchment-600">
              Versions, tool paths, settings and the last 200 log lines. Home
              paths are shortened; tokens and page text are never included.
            </p>
          </section>

          <section className="panel p-4">
            <div className="flex items-center gap-2">
              <CircuitBoard className="h-3.5 w-3.5 text-vermilion-300/80" strokeWidth={1.8} />
              <span className="text-[13px] font-semibold text-parchment-200">
                What the backend owns
              </span>
            </div>
            <ul className="mt-3 space-y-2">
              {ENGINEERING.map((item) => (
                <li key={item} className="flex items-start gap-2">
                  <span className="mt-[7px] h-1 w-1 shrink-0 bg-vermilion-500" />
                  <span className="text-[11.5px] leading-relaxed text-parchment-500">
                    {item}
                  </span>
                </li>
              ))}
            </ul>
          </section>

          <section className="panel p-4">
            <div className="flex items-center gap-2">
              <Scale className="h-3.5 w-3.5 text-vermilion-300/80" strokeWidth={1.8} />
              <span className="text-[13px] font-semibold text-parchment-200">
                Responsible use
              </span>
            </div>
            <p className="mt-2 text-[11.5px] leading-relaxed text-parchment-500">
              NovelForge adds its own resolver, transport policy and verification
              around maintained third-party engines. Download only material you
              are allowed to keep, follow each source's terms, and respect the
              law where you live. No DRM circumvention is included or supported.
            </p>
            <p className="mt-2 text-[11.5px] leading-relaxed text-parchment-600">
              The engines are maintained by their own communities; NovelForge
              ships no catalogue data and makes no claim about availability.
            </p>
          </section>
        </div>
      </div>
    </div>
  );
}

function RuntimeRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <dt className="text-[10.5px] font-semibold uppercase tracking-[0.13em] text-parchment-600">
        {label}
      </dt>
      <dd className="truncate font-mono text-[11.5px] tabular-nums text-parchment-300">
        {value}
      </dd>
    </div>
  );
}
