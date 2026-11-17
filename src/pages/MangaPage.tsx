import { useEffect, useMemo, useState } from "react";
import {
  ArrowRight,
  Check,
  ChevronDown,
  ClipboardCopy,
  ExternalLink,
  Filter,
  Images,
  Languages,
  Link2,
  Loader2,
  RefreshCw,
  Search,
  ShieldAlert,
} from "lucide-react";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import type { MangaSource } from "@/types";
import { cn } from "@/lib/utils";
import { chapterHint, parseChapterSpec } from "@/lib/chapters";
import { PageHeader } from "@/components/ui/PageHeader";
import { CrawlOptions } from "@/components/novels/CrawlOptions";

type UrlStatus =
  | { state: "empty" }
  | { state: "invalid"; message: string }
  | { state: "valid"; host: string; matched: MangaSource | null };

export function MangaPage({
  manager,
  onOpenSettings,
}: {
  manager: DownloadManager;
  onOpenSettings: () => void;
}) {
  const [url, setUrl] = useState("");
  const [chapters, setChapters] = useState("all");
  const [throttle, setThrottle] = useState(manager.settings?.mangaThrottle ?? true);
  const [sourceQuery, setSourceQuery] = useState("");
  const [language, setLanguage] = useState("all");
  const [showAdult, setShowAdult] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);

  const selection = parseChapterSpec(chapters);
  const sources = manager.mangaSources?.sources ?? [];

  useEffect(() => {
    void manager.loadMangaSources();
  }, [manager.loadMangaSources]);

  useEffect(() => {
    if (!manager.settings) return;
    setThrottle(manager.settings.mangaThrottle);
  }, [manager.settings]);

  const validation = useMemo<UrlStatus>(() => {
    const trimmed = url.trim();
    if (!trimmed) return { state: "empty" };
    try {
      const parsed = new URL(trimmed);
      if (!matches(parsed.protocol, ["http:", "https:"])) {
        return { state: "invalid", message: "Only http(s) links are accepted." };
      }
      if (!parsed.hostname) {
        return { state: "invalid", message: "The link has no hostname." };
      }
      const host = parsed.hostname.replace(/^www\./, "").toLowerCase();
      const matched =
        sources.find((source) => hostOf(source.url) === host) ?? null;
      return { state: "valid", host, matched };
    } catch {
      return {
        state: "invalid",
        message: "Paste a complete page URL, including https://",
      };
    }
  }, [sources, url]);

  const languages = useMemo(
    () => [...new Set(sources.map((source) => source.language))].sort(),
    [sources],
  );

  const adultCount = useMemo(
    () => sources.filter(isAdultSource).length,
    [sources],
  );

  const filteredSources = useMemo(() => {
    const needle = sourceQuery.trim().toLowerCase();
    return sources.filter((source) => {
      if (!showAdult && isAdultSource(source)) return false;
      if (language !== "all" && source.language !== language) return false;
      if (!needle) return true;
      return (
        source.name.toLowerCase().includes(needle) ||
        source.url.toLowerCase().includes(needle) ||
        source.language.toLowerCase().includes(needle)
      );
    });
  }, [language, sourceQuery, showAdult, sources]);

  const format = manager.settings?.mangaFormat ?? "cbz";

  async function copySource(source: MangaSource) {
    try {
      await navigator.clipboard.writeText(source.url);
      setCopied(source.url);
      window.setTimeout(() => setCopied((current) => (current === source.url ? null : current)), 1600);
    } catch {
      manager.showNotice(
        "error",
        "Clipboard unavailable",
        `Open ${source.url} manually.`,
      );
    }
  }

  async function start() {
    if (validation.state !== "valid") {
      manager.showNotice(
        "error",
        "Page URL required",
        validation.state === "invalid"
          ? validation.message
          : "Paste a page URL from a gallery-dl manga extractor.",
      );
      return;
    }
    if (!selection.valid) {
      manager.showNotice(
        "error",
        "Check the chapter selection",
        selection.error ?? undefined,
      );
      return;
    }

    const started = await manager.start("manga", url.trim(), {
      chapters: selection.normalized,
      throttle,
    });
    if (started) setUrl("");
  }

  return (
    <div className="space-y-4">
      <PageHeader
        eyebrow="Collection 02 / Manga"
        title="Manga / Manhwa / Manhua"
        description="gallery-dl accepts direct page links. Pick a manga extractor from its live catalogue, paste the title page URL, and choose exactly which chapters to pack."
        actions={
          <>
            <span className="chip">{sources.length > 0 ? `${sources.length} sources` : "gallery-dl catalogue"}</span>
            <span className="chip chip-vermilion">{format.toUpperCase()}</span>
          </>
        }
      />

      <div className="grid grid-cols-1 gap-3.5 xl:grid-cols-[minmax(0,1fr)_340px]">
        <section className="panel overflow-hidden">
          <div className="flex items-center justify-between border-b border-parchment-100/8 px-4 py-2.5">
            <div className="flex items-center gap-2">
              <Images className="h-3 w-3 text-vermilion-400" strokeWidth={1.9} />
              <span className="section-label">Source page</span>
            </div>
            <button
              type="button"
              onClick={onOpenSettings}
              className="text-[10px] font-semibold uppercase tracking-[0.12em] text-parchment-500 transition-colors hover:text-vermilion-200"
            >
              Container · {format.toUpperCase()}
            </button>
          </div>

          <div className="p-4">
            <form
              onSubmit={(event) => {
                event.preventDefault();
                void start();
              }}
              className="flex flex-col gap-2 sm:flex-row"
            >
              <div
                className={cn(
                  "field h-[50px] min-w-0 flex-1",
                  validation.state === "invalid" && "border-vermilion-500/55",
                )}
              >
                <Link2
                  className={cn(
                    "h-4 w-4 shrink-0",
                    validation.state === "valid"
                      ? "text-jade-400"
                      : validation.state === "invalid"
                        ? "text-vermilion-400"
                        : "text-parchment-500",
                  )}
                  strokeWidth={1.7}
                />
                <input
                  value={url}
                  onChange={(event) => setUrl(event.target.value)}
                  placeholder="https://mangadex.org/title/…"
                  aria-label="Manga page URL"
                  spellCheck={false}
                  autoComplete="off"
                  className="text-[14px]"
                />
                {url && (
                  <button
                    type="button"
                    onClick={() => setUrl("")}
                    className="shrink-0 text-[10px] uppercase tracking-[0.1em] text-parchment-600 transition-colors hover:text-parchment-300"
                  >
                    Clear
                  </button>
                )}
              </div>
              <button
                type="submit"
                disabled={manager.starting}
                className="btn-primary h-[50px] w-full shrink-0 gap-2 px-5 sm:w-auto"
              >
                {manager.starting ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
                ) : (
                  <ArrowRight className="h-3.5 w-3.5" strokeWidth={1.9} />
                )}
                Start download
              </button>
            </form>

            <UrlFeedback validation={validation} />

            <div className="mt-4 border-t border-parchment-100/7 pt-4">
              <CrawlOptions
                chapters={chapters}
                onChaptersChange={setChapters}
                throttle={throttle}
                onThrottleChange={setThrottle}
              />
            </div>

            <div className="mt-4 grid gap-3 border-t border-parchment-100/7 pt-4 sm:grid-cols-3">
              <Fact label="Chapters" value={chapterHint(selection)} />
              <Fact
                label="Container"
                value={
                  format === "zip"
                    ? "Plain ZIP archive"
                    : "CBZ comic archive"
                }
              />
              <Fact label="Politeness" value={throttle ? "Request delay" : "Maximum speed"} />
            </div>
          </div>
        </section>

        <section className="panel flex max-h-[460px] min-h-0 flex-col overflow-hidden sm:max-h-[500px] xl:max-h-[500px]">
          <div className="border-b border-parchment-100/8 px-4 py-2.5">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Filter className="h-3 w-3 text-vermilion-400" strokeWidth={1.9} />
                <span className="section-label">gallery-dl extractors</span>
              </div>
              <button
                type="button"
                onClick={() => void manager.loadMangaSources()}
                disabled={manager.sourcesLoading}
                aria-label="Refresh manga sources"
                title="Refresh source catalogue"
                className="icon-button h-6 w-6"
              >
                <RefreshCw
                  className={cn("h-3 w-3", manager.sourcesLoading && "animate-spin")}
                  strokeWidth={1.8}
                />
              </button>
            </div>
            <p className="mt-1.5 text-[10.5px] leading-relaxed text-parchment-600">
              Links must come from a supported source. Copy a base URL, then
              open the series page you want.
            </p>
          </div>

          <div className="space-y-2 border-b border-parchment-100/7 px-4 py-2.5">
            <div className="field h-8 gap-2 px-2.5">
              <Search className="h-3 w-3 text-parchment-500" strokeWidth={1.8} />
              <input
                value={sourceQuery}
                onChange={(event) => setSourceQuery(event.target.value)}
                placeholder="Filter sources…"
                aria-label="Filter manga sources"
                spellCheck={false}
                className="text-[12px]"
              />
            </div>

            <div className="flex items-center gap-1.5">
              <div className="relative min-w-0 flex-1">
                <Languages className="pointer-events-none absolute left-2 top-1/2 h-3 w-3 -translate-y-1/2 text-parchment-500" strokeWidth={1.8} />
                <select
                  value={language}
                  onChange={(event) => setLanguage(event.target.value)}
                  aria-label="Filter sources by language"
                  className="select-control h-7 w-full appearance-none pl-7 pr-6 text-[11px]"
                >
                  <option value="all" className="bg-[#121214]">All languages</option>
                  {languages.map((value) => (
                    <option key={value} value={value} className="bg-[#121214]">
                      {value}
                    </option>
                  ))}
                </select>
                <ChevronDown className="pointer-events-none absolute right-2 top-1/2 h-2.5 w-2.5 -translate-y-1/2 text-parchment-500" />
              </div>
              <button
                type="button"
                onClick={() => setShowAdult((current) => !current)}
                aria-pressed={showAdult}
                className={cn(
                  "flex h-7 shrink-0 items-center gap-1.5 border px-2 text-[9.5px] font-semibold uppercase tracking-[0.08em] transition-colors",
                  showAdult
                    ? "border-vermilion-500/40 bg-vermilion-500/10 text-vermilion-200"
                    : "border-parchment-100/10 text-parchment-500 hover:text-parchment-200",
                )}
                title={`${adultCount} tagged adult sources`}
              >
                <ShieldAlert className="h-3 w-3" strokeWidth={1.8} />
                18+
              </button>
            </div>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto">
            {manager.sourcesLoading && sources.length === 0 ? (
              <div className="space-y-1 p-2">
                {[0, 1, 2, 3, 4, 5].map((row) => (
                  <div
                    key={row}
                    className="h-[46px] animate-pulse-soft border border-parchment-100/[0.04] bg-parchment-100/[0.018]"
                  />
                ))}
              </div>
            ) : filteredSources.length === 0 ? (
              <div className="px-5 py-9 text-center">
                <div className="text-[12.5px] font-medium text-parchment-300">
                  No sources in this view
                </div>
                <p className="mt-1 text-[11px] leading-relaxed text-parchment-600">
                  {sources.length === 0
                    ? "The gallery-dl extractor list could not be read. Use Guardrails to check the gallery-dl binary."
                    : "Clear the filter or enable 18+ tagged sources."}
                </p>
              </div>
            ) : (
              filteredSources.map((source) => (
                <div
                  key={`${source.language}-${source.url}`}
                  className="group flex items-center gap-2 border-b border-parchment-100/[0.05] px-3 py-2 last:border-0 transition-colors hover:bg-parchment-100/[0.02]"
                >
                  <span
                    className={cn(
                      "h-1 w-1 shrink-0",
                      source.tags.some((tag) => /broken/i.test(tag))
                        ? "bg-vermilion-500"
                        : "bg-jade-500/70",
                    )}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <span className="truncate text-[12px] font-medium text-parchment-200">
                        {source.name}
                      </span>
                      {source.tags.map((tag) => (
                        <span
                          key={tag}
                          className={cn(
                            "shrink-0 border px-1 py-[1px] text-[9px] uppercase tracking-[0.06em]",
                            /broken/i.test(tag)
                              ? "border-vermilion-500/25 text-vermilion-300"
                              : "border-parchment-100/10 text-parchment-600",
                          )}
                        >
                          {tag}
                        </span>
                      ))}
                    </div>
                    <div className="mt-0.5 flex items-center gap-1.5 text-[9.5px] uppercase tracking-[0.08em] text-parchment-600">
                      <span>{source.language}</span>
                      <span className="text-parchment-700">·</span>
                      <span className="truncate font-mono normal-case tracking-normal">
                        {hostOf(source.url)}
                      </span>
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick={() => void copySource(source)}
                    aria-label={`Copy ${source.name} URL`}
                    title={`Copy ${source.url}`}
                    className="flex h-6 w-6 shrink-0 items-center justify-center text-parchment-600 opacity-0 transition-all hover:text-vermilion-200 group-hover:opacity-100 focus-visible:opacity-100"
                  >
                    {copied === source.url ? (
                      <Check className="h-3 w-3 text-jade-300" strokeWidth={2.2} />
                    ) : (
                      <ClipboardCopy className="h-3 w-3" strokeWidth={1.8} />
                    )}
                  </button>
                </div>
              ))
            )}
          </div>

          <div className="border-t border-parchment-100/7 px-4 py-2">
            <div className="flex items-center justify-between text-[9.5px] uppercase tracking-[0.1em] text-parchment-600">
              <span>{filteredSources.length} shown</span>
              <span className="flex items-center gap-1">
                <ExternalLink className="h-2.5 w-2.5" strokeWidth={1.8} />
                Copy base URLs
              </span>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}

function UrlFeedback({ validation }: { validation: UrlStatus }) {
  if (validation.state === "empty") {
    return (
      <p className="mt-2 text-[10.5px] leading-relaxed text-parchment-600">
        gallery-dl downloads a series page, not a search page. Pick the source
        below and paste the title URL.
      </p>
    );
  }

  if (validation.state === "invalid") {
    return (
      <p className="mt-2 flex items-center gap-1.5 text-[10.5px] text-vermilion-300">
        <ShieldAlert className="h-3 w-3" strokeWidth={1.9} />
        {validation.message}
      </p>
    );
  }

  return (
    <div className="mt-2 flex flex-wrap items-center gap-2 text-[10.5px]">
      {validation.matched ? (
        <>
          <span className="flex items-center gap-1.5 text-jade-300">
            <Check className="h-3 w-3" strokeWidth={2.2} />
            Recognised source — {validation.matched.name}
          </span>
          <span className="chip">{validation.matched.language}</span>
          <span className="font-mono text-[10px] text-parchment-600">{validation.host}</span>
        </>
      ) : (
        <>
          <span className="flex items-center gap-1.5 text-gold-300">
            <ShieldAlert className="h-3 w-3" strokeWidth={1.9} />
            Host not found in the gallery-dl catalogue
          </span>
          <span className="text-parchment-600">
            The downloader will still try; verify the exact source before relying on it.
          </span>
        </>
      )}
    </div>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-[9.5px] font-semibold uppercase tracking-[0.16em] text-parchment-600">
        {label}
      </div>
      <div className="mt-1 text-[11.5px] leading-relaxed text-parchment-300">{value}</div>
    </div>
  );
}

function hostOf(source: string): string {
  try {
    return new URL(source).hostname.replace(/^www\./, "").toLowerCase();
  } catch {
    return source.replace(/^https?:\/\//, "").replace(/\/$/, "").toLowerCase();
  }
}

function isAdultSource(source: MangaSource): boolean {
  return source.tags.some((tag) =>
    /(hentai|nsfw|adult|smut|18\+)/i.test(tag),
  );
}

function matches(value: string, options: string[]): boolean {
  return options.includes(value);
}
