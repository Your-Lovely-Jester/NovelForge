import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDownAZ,
  ArrowRight,
  BookOpen,
  ChevronDown,
  Copy,
  CornerDownLeft,
  Globe2,
  Languages,
  Loader2,
  Radio,
  Search,
  Sparkles,
  X,
} from "lucide-react";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import { errorMessage } from "@/hooks/useDownloadManager";
import type {
  CatalogueSource,
  ExistingDownload,
  NovelChapterIndex,
  NovelMetadata,
  NovelResult,
  ResolvedTitle,
  SourceSearchFailure,
} from "@/types";
import { api } from "@/lib/tauri";
import { cn, formatBytes } from "@/lib/utils";
import { parseChapterSpec } from "@/lib/chapters";
import { hostLabel, novelFacts, resultPath } from "@/lib/novels";
import {
  confidencePercent,
  confidenceTone,
  matchTypeLabel,
  needsConfirmation,
} from "@/lib/resolver";
import {
  estimateDownloadBytes,
  shouldWarnLargeDownload,
  type SizeEstimate,
} from "@/lib/storage";
import { TitlePlate } from "@/components/ui/TitlePlate";
import { CrawlOptions } from "@/components/novels/CrawlOptions";

const EXAMPLES = ["reverend insanity", "master of gu", "author: Gu Zhen Ren"];
const METADATA_CONCURRENCY = 4;
const METADATA_TIMEOUT_MS = 20_000;
const CHAPTER_TIMEOUT_MS = 300_000;

export function NovelSearchPanel({
  manager,
  initialQuery = "",
  searchNonce = 0,
}: {
  manager: DownloadManager;
  initialQuery?: string;
  searchNonce?: number;
}) {
  const [query, setQuery] = useState(initialQuery);
  const [results, setResults] = useState<NovelResult[] | null>(null);
  const [searched, setSearched] = useState("");
  const [searching, setSearching] = useState(false);
  const [catalogue, setCatalogue] = useState<CatalogueSource[]>([]);
  const [sourceChoice, setSourceChoice] = useState("novelforge");
  const [resultSource, setResultSource] = useState("all");
  const [originFilter, setOriginFilter] = useState("all");
  const [searchFailures, setSearchFailures] = useState<SourceSearchFailure[]>([]);
  const [searchedSources, setSearchedSources] = useState<string[]>([]);
  const [searchCache, setSearchCache] = useState<{ cached: boolean; age: number | null }>({
    cached: false,
    age: null,
  });
  const [sort, setSort] = useState<"relevance" | "title">("relevance");
  const [selectedUrl, setSelectedUrl] = useState<string | null>(null);
  const [resolvedMatch, setResolvedMatch] = useState<ResolvedTitle | null>(null);
  const [resolving, setResolving] = useState(false);
  const [resolveMode, setResolveMode] = useState<"auto" | "author">("auto");
  const [chapters, setChapters] = useState("all");
  const [throttle, setThrottle] = useState(true);
  const [metadata, setMetadata] = useState<Record<string, NovelMetadata>>({});
  const [metadataLoading, setMetadataLoading] = useState<Record<string, boolean>>({});
  const [metadataFailed, setMetadataFailed] = useState<Record<string, boolean>>({});
  const [chapterIndex, setChapterIndex] = useState<NovelChapterIndex | null>(null);
  const [chapterIndexLoading, setChapterIndexLoading] = useState(false);
  const [chapterIndexError, setChapterIndexError] = useState<string | null>(null);
  const [indexProgress, setIndexProgress] = useState<{
    page: number;
    pages: number;
  } | null>(null);
  const [chapterNonce, setChapterNonce] = useState(0);
  const [duplicatePrompt, setDuplicatePrompt] = useState<{
    url: string;
    existing: ExistingDownload;
  } | null>(null);
  const [duplicateBusy, setDuplicateBusy] = useState(false);
  const [largePrompt, setLargePrompt] = useState<{
    url: string;
    title: string;
    chapters: number;
    estimate: SizeEstimate;
  } | null>(null);

  const lastNonce = useRef(0);
  const hydrationToken = useRef(0);
  const chapterToken = useRef(0);
  const refreshNextIndex = useRef(false);
  const selection = parseChapterSpec(chapters);
  const storage = manager.library.storage;

  const resultSources = useMemo(
    () => Array.from(new Set((results ?? []).map((result) => result.source))).sort(),
    [results],
  );
  const originFilterLabel =
    originFilter === "ko"
      ? "Korean-origin"
      : originFilter === "zh"
        ? "Chinese-origin"
        : originFilter === "ja"
          ? "Japanese-origin"
          : "any origin";

  const catalogueByHost = useMemo(
    () => new Map(catalogue.map((entry) => [entry.host, entry])),
    [catalogue],
  );
  // Notes only for sources that actually answered this search, so the panel
  // stays quiet until a source-specific caveat is relevant.
  const availabilityNotes = useMemo(
    () =>
      searchedSources
        .map((host) => catalogueByHost.get(hostLabel(host)))
        .filter(
          (entry): entry is CatalogueSource =>
            Boolean(entry?.availabilityNote),
        ),
    [catalogueByHost, searchedSources],
  );

  const visible = useMemo(() => {
    let items = results ?? [];
    if (resultSource !== "all") {
      items = items.filter((result) => result.source === resultSource);
    }
    if (originFilter !== "all") {
      items = items.filter((result) =>
        catalogueByHost
          .get(result.source)
          ?.originLanguages.includes(originFilter),
      );
    }
    if (sort === "title") {
      items = [...items].sort((left, right) =>
        left.title.localeCompare(right.title, undefined, { sensitivity: "base" }),
      );
    }
    return items;
  }, [catalogueByHost, originFilter, results, sort, resultSource]);

  const searchableCatalogue = useMemo(
    () => catalogue.filter((entry) => entry.search && !entry.manga),
    [catalogue],
  );
  const directCatalogue = useMemo(
    // 289 of the 338 enabled sources cannot search and are not NovelForge
    // profiles. Listing all of them makes the picker unusable; direct links to
    // them still work when pasted into the search field.
    () => catalogue.filter((entry) => !entry.search && entry.novelforgeId !== null && !entry.manga),
    [catalogue],
  );
  const selectedEntry = useMemo(
    () =>
      sourceChoice === "novelforge" || sourceChoice === "all"
        ? null
        : catalogue.find((entry) => entry.url === sourceChoice) ?? null,
    [catalogue, sourceChoice],
  );
  const searchBlocked = Boolean(selectedEntry && !selectedEntry.search);
  const sourceNotes = useMemo(() => {
    const notes = new Map<string, string>();
    if (selectedEntry?.availabilityNote) {
      notes.set(selectedEntry.host, selectedEntry.availabilityNote);
    }
    for (const entry of availabilityNotes) {
      if (entry.availabilityNote) {
        notes.set(entry.host, entry.availabilityNote);
      }
    }
    return Array.from(notes.entries());
  }, [availabilityNotes, selectedEntry]);
  const sourceLabel =
    sourceChoice === "novelforge"
      ? "NovelForge sources"
      : sourceChoice === "all"
        ? `all ${searchableCatalogue.length} searchable sources`
        : selectedEntry?.host ?? "selected source";

  useEffect(() => {
    if (resultSource !== "all" && !resultSources.includes(resultSource)) {
      setResultSource("all");
    }
  }, [resultSource, resultSources]);

  useEffect(() => {
    let active = true;
    void api
      .listSourceCatalogue()
      .then((rows) => {
        if (active) setCatalogue(rows);
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, []);

  const selected =
    visible.find((result) => result.url === selectedUrl) ?? null;

  const hasFeed = results !== null || resolvedMatch !== null;

  const hydrateOne = useCallback(async (url: string, token: number) => {
    setMetadataLoading((current) => ({ ...current, [url]: true }));
    try {
      const value = await withTimeout(
        api.fetchNovelMetadata(url),
        METADATA_TIMEOUT_MS,
      );
      if (hydrationToken.current !== token) return;
      setMetadata((current) => ({ ...current, [url]: value }));
      setMetadataFailed((current) => {
        if (!(url in current)) return current;
        const next = { ...current };
        delete next[url];
        return next;
      });
    } catch {
      if (hydrationToken.current !== token) return;
      setMetadataFailed((current) => ({ ...current, [url]: true }));
    } finally {
      if (hydrationToken.current === token) {
        setMetadataLoading((current) => ({ ...current, [url]: false }));
      }
    }
  }, []);

  const hydrateResults = useCallback(
    async (rows: NovelResult[]) => {
      const token = ++hydrationToken.current;
      setMetadata({});
      setMetadataFailed({});
      setMetadataLoading({});
      let cursor = 0;
      const workers = Array.from(
        { length: Math.min(METADATA_CONCURRENCY, rows.length) },
        async () => {
          while (true) {
            const index = cursor;
            cursor += 1;
            if (index >= rows.length) return;
            await hydrateOne(rows[index].url, token);
          }
        },
      );
      await Promise.all(workers);
    },
    [hydrateOne],
  );

  useEffect(() => {
    if (!selectedUrl) {
      setIndexProgress(null);
      return;
    }
    let active = true;
    let dispose: (() => void) | undefined;
    void api
      .onIndexProgress((event) => {
        if (event.url !== selectedUrl) return;
        setIndexProgress({ page: event.page, pages: event.pages });
      })
      .then((off) => {
        if (active) dispose = off;
        else off();
      });
    return () => {
      active = false;
      dispose?.();
    };
  }, [selectedUrl]);

  useEffect(() => {
    if (!selectedUrl) {
      chapterToken.current += 1;
      setChapterIndex(null);
      setChapterIndexError(null);
      setChapterIndexLoading(false);
      return;
    }
    setIndexProgress(null);
    const token = ++chapterToken.current;
    const refresh = refreshNextIndex.current;
    refreshNextIndex.current = false;
    setChapterIndex(null);
    setChapterIndexError(null);
    setChapterIndexLoading(true);
    void withTimeout(api.fetchNovelChapters(selectedUrl, refresh), CHAPTER_TIMEOUT_MS)
      .then((index) => {
        if (chapterToken.current === token) setChapterIndex(index);
      })
      .catch((error) => {
        if (chapterToken.current === token) {
          setChapterIndexError(errorMessage(error));
        }
      })
      .finally(() => {
        if (chapterToken.current === token) setChapterIndexLoading(false);
      });
  }, [chapterNonce, selectedUrl]);

  async function runSearch(raw: string) {
    const trimmed = raw.trim();
    if (trimmed.length < 2) {
      manager.showNotice("error", "Search needs a title", "Type at least two characters.");
      return;
    }
    if (searchBlocked) {
      manager.showNotice(
        "error",
        "This source is direct-URL only",
        "Paste a page link from this source instead of searching its catalogue.",
      );
      return;
    }
    setSearching(true);
    setSelectedUrl(null);
    setResolvedMatch(null);
    setSearchFailures([]);
    try {
      const sources =
        sourceChoice === "novelforge"
          ? []
          : sourceChoice === "all"
            ? ["all"]
            : [sourceChoice];
      const response = await manager.searchNovels(trimmed, sources);
      if (!response) return;
      setResults(response.results);
      setSearched(response.query || trimmed);
      setSearchFailures(response.failures ?? []);
      setSearchedSources(response.searched ?? []);
      setSearchCache({
        cached: response.cached ?? false,
        age: response.cacheAgeSeconds ?? null,
      });
      setResultSource("all");
      setOriginFilter("all");
      void hydrateResults(response.results);
    } finally {
      setSearching(false);
    }
  }

  useEffect(() => {
    if (!searchNonce || searchNonce === lastNonce.current) return;
    const trimmed = initialQuery.trim();
    if (!trimmed) return;
    lastNonce.current = searchNonce;
    setQuery(initialQuery);
    void runSearch(initialQuery);
    // The nonce is the explicit trigger; query edits must not re-run the search.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchNonce, initialQuery]);

  function options(duplicate?: "update" | "copy") {
    return {
      chapters: selection.normalized,
      throttle,
      duplicate: duplicate ?? null,
    };
  }

  async function beginDownload(url: string, duplicate?: "update" | "copy") {
    setDuplicateBusy(true);
    const started = await manager.start("webnovel", url, options(duplicate));
    setDuplicateBusy(false);
    if (started) {
      setDuplicatePrompt(null);
      setSelectedUrl(null);
      setResolvedMatch(null);
      setChapters("all");
    }
    return started;
  }

  async function requestDownload(url: string, title?: string | null) {
    if (!selection.valid) {
      manager.showNotice(
        "error",
        "Check the chapter selection",
        selection.error ?? undefined,
      );
      return;
    }
    setDuplicateBusy(true);
    try {
      const existing = await api.findExistingDownload(url);
      if (existing) {
        setDuplicatePrompt({ url, existing });
        return;
      }
    } catch (error) {
      manager.showNotice("error", "Could not check your library", errorMessage(error));
      return;
    } finally {
      setDuplicateBusy(false);
    }

    const known = knownChapterCount(url);
    if (shouldWarnLargeDownload(selection.all, known)) {
      setLargePrompt({
        url,
        title: title?.trim() || metadata[url]?.title?.trim() || "this novel",
        chapters: known,
        estimate: estimateDownloadBytes(known, manager.library.novels ?? []),
      });
      return;
    }

    await beginDownload(url);
  }

  function knownChapterCount(url: string): number | null {
    if (selectedUrl === url && chapterIndex && chapterIndex.total > 0) {
      return chapterIndex.total;
    }
    const meta = metadata[url];
    if (meta?.chapterCount && meta.chapterCount > 0) {
      return meta.chapterCount;
    }
    return null;
  }

  async function resolveSource() {
    const trimmed = query.trim();
    if (trimmed.length < 2) {
      manager.showNotice("error", "Nothing to resolve", "Enter a title or author first.");
      return;
    }
    setResolving(true);
    setSelectedUrl(null);
    setResolvedMatch(null);
    try {
      const sources = sourceChoice === "novelforge" ? [] : [sourceChoice];
      const match = await manager.resolveTitle(
        "webnovel",
        trimmed,
        resolveMode,
        sources,
      );
      if (match) {
        setResolvedMatch(match);
        setSearched(trimmed);
        if (/^https?:\/\//i.test(match.source)) {
          void hydrateOne(match.source, hydrationToken.current);
        }
      }
    } finally {
      setResolving(false);
    }
  }

  function chooseAlternative(result: NovelResult) {
    if (!resolvedMatch) return;
    setResolvedMatch({
      ...resolvedMatch,
      title: result.title,
      source: result.url,
      detail: `Confirmed on ${result.source}`,
      confidence: 1,
      matchedBy: "title",
      alternatives: [],
    });
    void hydrateOne(result.url, hydrationToken.current);
  }

  async function startSelected() {
    if (!selected) return;
    await requestDownload(
      selected.url,
      metadata[selected.url]?.title ?? selected.title,
    );
  }

  async function startResolved() {
    if (!resolvedMatch) return;
    await requestDownload(
      resolvedMatch.source,
      metadata[resolvedMatch.source]?.title ?? resolvedMatch.title,
    );
  }

  function searchExample(value: string) {
    setQuery(value);
    void runSearch(value);
  }

  return (
    <div className="space-y-3">
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void runSearch(query);
        }}
        className="flex"
      >
        <div className="field h-[52px] min-w-0 flex-1 border-r-0">
          <Search className="h-[15px] w-[15px] shrink-0 text-parchment-500" strokeWidth={1.7} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search a title, alias, or author — reverend insanity, master of gu, author: Gu Zhen Ren…"
            aria-label="Search webnovels"
            spellCheck={false}
            autoComplete="off"
            className="text-[14.5px]"
          />
          {query && (
            <button
              type="button"
              onClick={() => setQuery("")}
              aria-label="Clear search"
              className="flex h-5 w-5 shrink-0 items-center justify-center text-parchment-600 transition-colors hover:text-parchment-200"
            >
              <X className="h-3.5 w-3.5" strokeWidth={1.8} />
            </button>
          )}
        </div>
        <button
          type="submit"
          disabled={searching}
          className="btn-primary h-[52px] w-[128px] shrink-0 gap-2 px-4"
        >
          {searching ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
          ) : (
            <CornerDownLeft className="h-3.5 w-3.5" strokeWidth={1.9} />
          )}
          Search
        </button>
      </form>

      <div className="flex flex-wrap items-center gap-1.5">
        <div className="relative">
          <Globe2 className="pointer-events-none absolute left-2.5 top-1/2 h-3 w-3 -translate-y-1/2 text-parchment-500" strokeWidth={1.8} />
          <select
            value={sourceChoice}
            onChange={(event) => setSourceChoice(event.target.value)}
            aria-label="Choose which source to search"
            className="select-control h-8 max-w-[280px] appearance-none pl-8 pr-7 text-[11.5px]"
          >
            <option value="novelforge" className="bg-[#121214]">
              NovelForge sources · NovelFire + NovelPhoenix
            </option>
            <option value="all" className="bg-[#121214]">
              All searchable sources ({searchableCatalogue.length})
            </option>
            {Array.from(new Set(searchableCatalogue.map((entry) => entry.language)))
              .sort()
              .map((language) => (
                <optgroup key={language} label={`Searchable · ${language}`}>
                  {searchableCatalogue
                    .filter((entry) => entry.language === language)
                    .map((entry) => (
                      <option key={entry.url} value={entry.url} className="bg-[#121214]">
                        {entry.host} · {entry.language}
                        {entry.translatedFromKorean ? " · Korean translations" : ""}
                        {entry.mtl ? " · MTL" : ""}
                        {entry.downloadReady ? " · NovelForge verified" : ""}
                      </option>
                    ))}
                </optgroup>
              ))}
            {directCatalogue.length > 0 && (
              <optgroup label="NovelForge direct-URL sources · no title search">
                {directCatalogue.map((entry) => (
                  <option key={entry.url} value={entry.url} className="bg-[#121214]">
                    {entry.host} · {entry.language}
                    {entry.translatedFromKorean ? " · Korean translations" : ""}
                    {entry.downloadReady ? " · verified" : ""}
                    {entry.mtl ? " · MTL" : ""}
                  </option>
                ))}
              </optgroup>
            )}
          </select>
          <ChevronDown className="pointer-events-none absolute right-2 top-1/2 h-2.5 w-2.5 -translate-y-1/2 text-parchment-500" />
        </div>

        <div className="relative">
          <Globe2 className="pointer-events-none absolute left-2.5 top-1/2 h-3 w-3 -translate-y-1/2 text-parchment-500" strokeWidth={1.8} />
          <select
            value={resultSource}
            onChange={(event) => setResultSource(event.target.value)}
            aria-label="Filter search results by source"
            className="select-control h-8 appearance-none pl-8 pr-7 text-[11.5px]"
          >
            <option value="all" className="bg-[#121214]">All result sources</option>
            {resultSources.map((source) => (
              <option key={source} value={source} className="bg-[#121214]">
                {hostLabel(source)}
              </option>
            ))}
          </select>
          <ChevronDown className="pointer-events-none absolute right-2 top-1/2 h-2.5 w-2.5 -translate-y-1/2 text-parchment-500" />
        </div>

        <div className="relative">
          <Languages className="pointer-events-none absolute left-2.5 top-1/2 h-3 w-3 -translate-y-1/2 text-parchment-500" strokeWidth={1.8} />
          <select
            value={originFilter}
            onChange={(event) => setOriginFilter(event.target.value)}
            aria-label="Filter search results by original language"
            title="Original language is a best-effort tag from the NovelForge source registry. Sources differ."
            className="select-control h-8 appearance-none pl-8 pr-7 text-[11.5px]"
          >
            <option value="all" className="bg-[#121214]">Any origin</option>
            <option value="ko" className="bg-[#121214]">Korean-origin · best effort</option>
            <option value="zh" className="bg-[#121214]">Chinese-origin</option>
            <option value="ja" className="bg-[#121214]">Japanese-origin</option>
          </select>
          <ChevronDown className="pointer-events-none absolute right-2 top-1/2 h-2.5 w-2.5 -translate-y-1/2 text-parchment-500" />
        </div>

        <div className="relative">
          <ArrowDownAZ className="pointer-events-none absolute left-2.5 top-1/2 h-3 w-3 -translate-y-1/2 text-parchment-500" strokeWidth={1.8} />
          <select
            value={sort}
            onChange={(event) => setSort(event.target.value as "relevance" | "title")}
            aria-label="Sort search results"
            className="select-control h-8 appearance-none pl-8 pr-7 text-[11.5px]"
          >
            <option value="relevance" className="bg-[#121214]">Best match</option>
            <option value="title" className="bg-[#121214]">A–Z</option>
          </select>
          <ChevronDown className="pointer-events-none absolute right-2 top-1/2 h-2.5 w-2.5 -translate-y-1/2 text-parchment-500" />
        </div>

        <span className="hidden items-center gap-1.5 pl-1 text-[9.5px] uppercase tracking-[0.14em] text-parchment-600 sm:flex">
          <Radio className="h-2.5 w-2.5 text-vermilion-400" strokeWidth={2} />
          Live search · {sourceLabel}
          {searchedSources.length > 0 && ` · ${searchedSources.length} answered`}
        </span>

        <div className="ml-auto flex h-8 items-center border border-parchment-100/10 bg-black/20">
          {(["auto", "author"] as const).map((mode) => (
            <button
              key={mode}
              type="button"
              onClick={() => setResolveMode(mode)}
              aria-pressed={resolveMode === mode}
              className={cn(
                "h-full px-2 text-[9.5px] font-semibold uppercase tracking-[0.12em] transition-colors",
                resolveMode === mode
                  ? "bg-vermilion-500/[0.14] text-vermilion-200"
                  : "text-parchment-500 hover:text-parchment-300",
              )}
              title={
                mode === "author"
                  ? "Read each source's author page for the full bibliography"
                  : "Search by title, alias or acronym"
              }
            >
              {mode === "author" ? "Author" : "Title"}
            </button>
          ))}
        </div>

        <button
          type="button"
          onClick={() => void resolveSource()}
          disabled={resolving || query.trim().length < 2}
          title="Search the selected sources, verify aliases, then fall back to AniList and Wikipedia"
          className="flex h-8 items-center gap-1.5 border border-vermilion-500/30 px-2.5 text-[10.5px] font-semibold uppercase tracking-[0.12em] text-vermilion-300 transition-colors hover:border-vermilion-400/55 hover:bg-vermilion-500/[0.07] hover:text-vermilion-200 disabled:border-parchment-100/8 disabled:text-parchment-600"
        >
          {resolving ? (
            <Loader2 className="h-3 w-3 animate-spin" strokeWidth={2} />
          ) : (
            <Sparkles className="h-3 w-3" strokeWidth={1.9} />
          )}
          Resolve source
        </button>
      </div>

      {searchBlocked && (
        <p className="border border-amber-500/25 bg-amber-500/[0.06] px-3 py-2 text-[11px] leading-relaxed text-amber-200/90">
          {selectedEntry?.host} is listed by lncrawl for direct links only. Pick a
          searchable source above, or paste a page URL into the search field to
          download from it.
        </p>
      )}

      {storage?.overQuota && (
        <p className="border border-vermilion-500/30 bg-vermilion-500/[0.06] px-3 py-2 text-[11px] leading-relaxed text-vermilion-200">
          Library quota reached ({formatBytes(storage.libraryBytes)} of{" "}
          {formatBytes(storage.quotaBytes)}). New downloads are refused until you
          delete partial output in Library or raise the limit in Settings.
        </p>
      )}

      {sourceNotes.length > 0 && (
        <div className="border border-parchment-100/10 bg-parchment-100/[0.025] px-3 py-2">
          <p className="text-[10px] font-semibold uppercase tracking-[0.14em] text-parchment-400">
            Source availability
          </p>
          <ul className="mt-1 space-y-1">
            {sourceNotes.map(([host, note]) => (
              <li key={host} className="text-[11px] leading-relaxed text-parchment-500">
                <span className="font-medium text-parchment-300">{hostLabel(host)}</span>
                {" — "}
                {note}
              </li>
            ))}
          </ul>
        </div>
      )}

      {searchFailures.length > 0 && (
        <div className="border border-amber-500/25 bg-amber-500/[0.05] px-3 py-2">
          <p className="text-[10px] font-semibold uppercase tracking-[0.14em] text-amber-300">
            {searchFailures.length} source{searchFailures.length === 1 ? "" : "s"} failed
          </p>
          <ul className="mt-1 space-y-1">
            {searchFailures.slice(0, 6).map((failure) => (
              <li key={`${failure.source}-${failure.reason}`} className="text-[11px] leading-relaxed text-amber-100/80">
                <span className="font-medium text-amber-200">{failure.label}</span>
                {" — "}
                {failure.reason}
              </li>
            ))}
          </ul>
          {searchFailures.length > 6 && (
            <p className="mt-1 text-[10px] text-amber-200/60">
              and {searchFailures.length - 6} more
            </p>
          )}
        </div>
      )}

      {resolvedMatch && (
        <ResolvedPanel
          match={resolvedMatch}
          meta={metadata[resolvedMatch.source] ?? null}
          starting={manager.starting}
          chapters={chapters}
          onChaptersChange={setChapters}
          throttle={throttle}
          onThrottleChange={setThrottle}
          onStart={() => void startResolved()}
          onChooseAlternative={chooseAlternative}
          onClose={() => setResolvedMatch(null)}
        />
      )}

      {searching && results === null && <SearchSkeleton />}

      {!searching && !hasFeed && (
        <SearchIntro onExample={searchExample} onResolve={() => void resolveSource()} />
      )}

      {results !== null && (
        <section className="panel overflow-hidden">
          <div className="flex items-center justify-between gap-3 border-b border-parchment-100/8 px-4 py-2.5">
            <div className="flex min-w-0 items-center gap-2">
              <span
                className={cn(
                  "h-1.5 w-1.5 shrink-0 bg-vermilion-500",
                  searching && "animate-pulse-soft",
                )}
              />
              <span className="text-[9.5px] font-semibold uppercase tracking-[0.18em] text-parchment-300">
                {resolveMode === "author" ? "Title matches" : "Live results"}
              </span>
              <span className="font-mono text-[10px] tabular-nums text-parchment-600">
                {visible.length.toString().padStart(2, "0")}
              </span>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {searchCache.cached && (
                <span
                  className="font-mono text-[10px] tabular-nums text-parchment-600"
                  title="These source rows came from a short-lived disk cache."
                >
                  cached {formatCacheAge(searchCache.age)}
                </span>
              )}
              {searched && (
                <span className="max-w-[46vw] truncate text-[10px] uppercase tracking-[0.1em] text-parchment-600">
                  for “{searched}”
                </span>
              )}
            </div>
          </div>

          {visible.length === 0 ? (
            <div className="px-5 py-9 text-center">
              <div className="text-[13.5px] font-medium text-parchment-300">
                No matches in this view
              </div>
              <p className="mx-auto mt-1 max-w-[420px] text-[12px] leading-relaxed text-parchment-600">
                {results.length === 0 ? (
                  <>
                    No hit from {sourceLabel}. Catalogues differ: this search
                    asked {searchedSources.length > 0
                      ? searchedSources.map((source) => hostLabel(source)).join(", ")
                      : "the selected sources"}
                    . English aggregators may carry translations of Korean,
                    Chinese or Japanese novels, but they do not share one
                    catalogue, and some remove titles for copyright reasons.
                  </>
                ) : originFilter !== "all" ? (
                  `No ${originFilterLabel} match in this view. Origin tags are best effort; try another origin or clear the filter.`
                ) : (
                  "Try another source filter, or let the resolver widen the search."
                )}
              </p>
              <button
                type="button"
                onClick={() => void resolveSource()}
                disabled={resolving}
                className="btn-secondary mt-3"
              >
                <Sparkles className="h-3 w-3" strokeWidth={1.9} />
                Resolve canonical title
              </button>
            </div>
          ) : (
            visible.map((result, index) => {
              const facts = novelFacts(result);
              const entry = catalogueByHost.get(result.source);
              const active = result.url === selectedUrl;
              const meta = metadata[result.url];
              const loadingMeta = Boolean(metadataLoading[result.url]);
              const metaFailed = Boolean(metadataFailed[result.url]);
              const title = meta?.title?.trim() || result.title;
              const author = meta?.author ?? meta?.authors[0] ?? null;
              const tags = meta?.tags.length ? meta.tags.slice(0, 4) : facts.tags;
              const synopsis = meta?.synopsis?.trim() || null;
              const chapterCount = meta?.chapterCount ?? facts.chapters;
              const status = meta?.status ?? facts.status;
              const language = meta?.language ?? facts.language;
              return (
                <article
                  key={`${result.source}-${result.url}`}
                  className={cn(
                    "border-b border-parchment-100/[0.065] last:border-b-0",
                    active && "bg-vermilion-900/[0.12]",
                  )}
                >
                  <div className="flex items-start gap-3 px-4 py-3.5">
                    <span className="hidden w-5 shrink-0 pt-1 font-mono text-[10px] text-parchment-700 sm:block">
                      {(index + 1).toString().padStart(2, "0")}
                    </span>
                    {meta?.cover ? (
                      <img
                        src={meta.cover}
                        alt=""
                        loading="lazy"
                        className="h-[66px] w-[48px] shrink-0 rounded-[2px] border border-parchment-100/12 object-cover"
                      />
                    ) : (
                      <TitlePlate title={title} source={result.url} />
                    )}
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
                        <h3 className="font-display text-[22px] font-medium leading-[1.02] tracking-[0.003em] text-parchment-50">
                          {title}
                        </h3>
                        {status && <span className="chip">{status}</span>}
                        {entry && (
                          <span
                            className="chip"
                            title={`Site language reported by lncrawl: ${entry.language}`}
                          >
                            {entry.language.toUpperCase()} site
                          </span>
                        )}
                        {entry?.translatedFromKorean && (
                          <span
                            className="chip chip-vermilion"
                            title="Best-effort tag from the NovelForge source registry, not a guarantee that this title exists on every source."
                          >
                            Korean-origin · best effort
                          </span>
                        )}
                        {loadingMeta && (
                          <span className="inline-flex items-center gap-1 text-[9.5px] uppercase tracking-[0.1em] text-parchment-600">
                            <Loader2 className="h-2.5 w-2.5 animate-spin" strokeWidth={2} />
                            reading metadata
                          </span>
                        )}
                      </div>
                      <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] uppercase tracking-[0.09em] text-parchment-500">
                        <span className="inline-flex items-center gap-1.5 text-parchment-300">
                          <span className="h-1 w-1 bg-vermilion-400" />
                          {hostLabel(result.source)}
                        </span>
                        {author && (
                          <>
                            <span className="text-parchment-700">·</span>
                            <span>by {author}</span>
                          </>
                        )}
                        {language && (
                          <>
                            <span className="text-parchment-700">·</span>
                            <span>{language} novel</span>
                          </>
                        )}
                        {tags.map((tag) => (
                          <span key={tag} className="flex items-center gap-2">
                            <span className="text-parchment-700">·</span>
                            <span>{tag}</span>
                          </span>
                        ))}
                      </div>
                      <p className="mt-1.5 line-clamp-2 max-w-[620px] text-[11.5px] leading-relaxed text-parchment-500">
                        {synopsis ||
                          (metaFailed
                            ? "Metadata source blocked the request — title and URL are still available."
                            : resultPath(result.url))}
                      </p>
                    </div>

                    <div className="flex shrink-0 items-center gap-4">
                      <div className="hidden w-16 text-right sm:block">
                        <div className="font-display text-[20px] leading-none text-parchment-200">
                          {chapterCount
                            ? chapterCount.toLocaleString()
                            : loadingMeta
                              ? "…"
                              : "—"}
                        </div>
                        <div className="mt-1 text-[9px] font-medium uppercase tracking-[0.15em] text-parchment-600">
                          {chapterCount ? "chapters" : "indexed"}
                        </div>
                      </div>
                      <button
                        type="button"
                        onClick={() => {
                          setResolvedMatch(null);
                          setSelectedUrl(active ? null : result.url);
                        }}
                        aria-expanded={active}
                        className={cn(
                          "flex h-8 items-center gap-2 border px-3 text-[10.5px] font-semibold uppercase tracking-[0.12em] transition-colors",
                          active
                            ? "border-vermilion-500/55 bg-vermilion-500/10 text-vermilion-200"
                            : "border-parchment-100/14 text-parchment-300 hover:border-vermilion-500/45 hover:bg-vermilion-500/[0.06] hover:text-parchment-50",
                        )}
                      >
                        {active ? "Close" : "Select"}
                        {active ? (
                          <X className="h-3 w-3" strokeWidth={2} />
                        ) : (
                          <ArrowRight className="h-3 w-3" strokeWidth={1.8} />
                        )}
                      </button>
                    </div>
                  </div>

                  {active && (
                    <div className="border-t border-vermilion-500/20 bg-[#0a0a0c] px-4 py-3.5">
                      <div className="flex flex-wrap items-start justify-between gap-3">
                        <div className="flex min-w-0 items-start gap-3">
                          {meta?.cover && (
                            <img
                              src={meta.cover}
                              alt=""
                              className="h-[92px] w-[66px] shrink-0 rounded-[2px] border border-parchment-100/14 object-cover"
                            />
                          )}
                          <div className="min-w-0">
                            <div className="text-[9.5px] font-semibold uppercase tracking-[0.18em] text-vermilion-300/90">
                              Prepare download
                            </div>
                            <div className="mt-1 flex flex-wrap items-center gap-2">
                              <span className="truncate font-display text-[19px] text-parchment-50">
                                {title}
                              </span>
                              <span className="chip chip-vermilion">{hostLabel(result.source)}</span>
                              {status && <span className="chip">{status}</span>}
                              {entry?.translatedFromKorean && (
                                <span className="chip">Korean-origin · best effort</span>
                              )}
                            </div>
                            {author && (
                              <div className="mt-1 text-[10.5px] uppercase tracking-[0.08em] text-parchment-500">
                                by {author}
                                {chapterCount
                                  ? ` · ${chapterCount.toLocaleString()} chapters`
                                  : ""}
                              </div>
                            )}
                            {synopsis && (
                              <p className="mt-1.5 line-clamp-3 max-w-[620px] text-[11.5px] leading-relaxed text-parchment-400">
                                {synopsis}
                              </p>
                            )}
                            {tags.length > 0 && (
                              <div className="mt-1.5 flex flex-wrap gap-1.5">
                                {tags.map((tag) => (
                                  <span
                                    key={tag}
                                    className="border border-parchment-100/10 px-1.5 py-0.5 text-[9px] uppercase tracking-[0.1em] text-parchment-500"
                                  >
                                    {tag}
                                  </span>
                                ))}
                              </div>
                            )}
                            <div className="mt-1 max-w-[560px] truncate font-mono text-[10px] text-parchment-600">
                              {result.url}
                            </div>
                            {entry?.availabilityNote && (
                              <p className="mt-1.5 max-w-[560px] border-l border-amber-500/40 pl-2 text-[10.5px] leading-relaxed text-amber-200/80">
                                {entry.availabilityNote}
                              </p>
                            )}
                          </div>
                        </div>
                        <button
                          type="button"
                          onClick={() => void startSelected()}
                          disabled={
                            manager.starting ||
                            chapterIndexLoading ||
                            !selection.valid
                          }
                          className="btn-primary h-9 shrink-0 px-4"
                        >
                          {manager.starting ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
                          ) : (
                            <ArrowRight className="h-3.5 w-3.5" strokeWidth={1.9} />
                          )}
                          Start download
                        </button>
                      </div>

                      <ChapterIndexPanel
                        index={chapterIndex}
                        loading={chapterIndexLoading}
                        progress={chapterIndexLoading ? indexProgress : null}
                        error={chapterIndexError}
                        onRetry={() => setChapterNonce((value) => value + 1)}
                        onRefresh={() => {
                          refreshNextIndex.current = true;
                          setChapterNonce((value) => value + 1);
                        }}
                      />

                      <CrawlOptions
                        className="mt-3.5"
                        chapters={chapters}
                        onChaptersChange={setChapters}
                        throttle={throttle}
                        onThrottleChange={setThrottle}
                        throttleMode="rate-limit"
                      />
                      <p className="mt-2 text-[10px] leading-relaxed text-parchment-600">
                        The selection is de-duplicated, sorted and compressed before crawling.
                        Output is a structured folder with a cover, metadata.json, separate
                        chapter files and a bundled EPUB.
                      </p>
                    </div>
                  )}
                </article>
              );
            })
          )}
        </section>
      )}

      {duplicatePrompt && (
        <DuplicatePrompt
          existing={duplicatePrompt.existing}
          busy={duplicateBusy || manager.starting}
          onUpdate={() => void beginDownload(duplicatePrompt.url, "update")}
          onCopy={() => void beginDownload(duplicatePrompt.url, "copy")}
          onCancel={() => setDuplicatePrompt(null)}
        />
      )}

      {largePrompt && (
        <LargeDownloadPrompt
          title={largePrompt.title}
          chapters={largePrompt.chapters}
          estimate={largePrompt.estimate}
          libraryBytes={storage?.libraryBytes ?? 0}
          quotaBytes={storage?.quotaBytes ?? 0}
          busy={manager.starting}
          onCancel={() => setLargePrompt(null)}
          onStart={() => {
            const url = largePrompt.url;
            setLargePrompt(null);
            void beginDownload(url);
          }}
        />
      )}
    </div>
  );
}

function LargeDownloadPrompt({
  title,
  chapters,
  estimate,
  libraryBytes,
  quotaBytes,
  busy,
  onCancel,
  onStart,
}: {
  title: string;
  chapters: number;
  estimate: SizeEstimate;
  libraryBytes: number;
  quotaBytes: number;
  busy: boolean;
  onCancel: () => void;
  onStart: () => void;
}) {
  const projected = libraryBytes + estimate.bytes;
  const overQuota = quotaBytes > 0 && projected >= quotaBytes;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="large-download-title"
    >
      <div className="w-full max-w-[500px] border border-parchment-100/14 bg-[#0c0c0e] p-5 shadow-2xl">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="text-[9.5px] font-semibold uppercase tracking-[0.18em] text-gold-300">
              Large all-chapters download
            </div>
            <h2
              id="large-download-title"
              className="mt-1 truncate font-display text-[21px] leading-tight text-parchment-50"
            >
              {title}
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
          Every chapter will be fetched:{" "}
          <span className="text-parchment-200">
            {chapters.toLocaleString()} chapters
          </span>
          . A partial folder is kept if the run fails, so it can be retried.
        </p>

        <div className="mt-3 grid grid-cols-2 gap-2">
          <div className="border border-parchment-100/10 bg-black/25 px-3 py-2">
            <div className="text-[9.5px] uppercase tracking-[0.12em] text-parchment-600">
              Estimated size
            </div>
            <div className="mt-1 font-mono text-[15px] tabular-nums text-parchment-200">
              ≈ {formatBytes(estimate.bytes)}
            </div>
            <div className="mt-0.5 text-[10px] text-parchment-600">
              {estimate.basis === "library"
                ? `from your library · ${formatBytes(estimate.bytesPerChapter)}/chapter`
                : `rough estimate · ${formatBytes(estimate.bytesPerChapter)}/chapter`}
            </div>
          </div>
          <div className="border border-parchment-100/10 bg-black/25 px-3 py-2">
            <div className="text-[9.5px] uppercase tracking-[0.12em] text-parchment-600">
              Library after
            </div>
            <div className="mt-1 font-mono text-[15px] tabular-nums text-parchment-200">
              ≈ {formatBytes(projected)}
            </div>
            <div className="mt-0.5 text-[10px] text-parchment-600">
              {quotaBytes > 0
                ? `quota ${formatBytes(quotaBytes)}`
                : "no quota configured"}
            </div>
          </div>
        </div>

        <p className="mt-2 text-[10.5px] leading-relaxed text-parchment-600">
          Size is an estimate; image-heavy chapters and the bundled EPUB change
          the final result. The exact number appears on the novel card after the
          import.
        </p>

        {overQuota && (
          <p className="mt-2 border border-vermilion-500/30 bg-vermilion-500/[0.06] px-2.5 py-2 text-[11px] leading-relaxed text-vermilion-200">
            This run is estimated to push the library past its quota. The start
            check uses the bytes already on disk, so it may still begin, but the
            library will report over quota afterwards.
          </p>
        )}

        <div className="mt-4 flex items-center justify-end gap-2">
          <button type="button" onClick={onCancel} disabled={busy} className="btn-secondary">
            Cancel
          </button>
          <button type="button" onClick={onStart} disabled={busy} className="btn-primary">
            {busy ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
            ) : (
              <ArrowRight className="h-3.5 w-3.5" strokeWidth={1.9} />
            )}
            Start download
          </button>
        </div>
      </div>
    </div>
  );
}

function DuplicatePrompt({
  existing,
  busy,
  onUpdate,
  onCopy,
  onCancel,
}: {
  existing: ExistingDownload;
  busy: boolean;
  onUpdate: () => void;
  onCopy: () => void;
  onCancel: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="duplicate-title"
    >
      <div className="w-full max-w-[500px] border border-parchment-100/14 bg-[#0c0c0e] p-5 shadow-2xl">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="text-[9.5px] font-semibold uppercase tracking-[0.18em] text-vermilion-300">
              Already in your library
            </div>
            <h2
              id="duplicate-title"
              className="mt-1 truncate font-display text-[21px] text-parchment-50"
            >
              {existing.title}
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
          This source URL already has a structured download.{" "}
          <span className="text-parchment-200">Update existing</span> reuses that folder and
          preserves chapters that are not part of this run.{" "}
          <span className="text-parchment-200">Keep both</span> creates an explicit copy.
        </p>

        <div className="mt-3 border border-parchment-100/10 bg-black/25 px-3 py-2">
          <div className="truncate font-mono text-[10px] text-parchment-500" title={existing.folder}>
            {existing.folder}
          </div>
          <div className="mt-1.5 flex flex-wrap gap-x-3 gap-y-1 text-[10px] uppercase tracking-[0.1em]">
            <span className="font-mono tabular-nums text-parchment-400">
              {existing.downloadedChapters.toLocaleString()} /{" "}
              {existing.chapterCount.toLocaleString()} chapters
            </span>
            <span className={existing.complete ? "text-emerald-300" : "text-gold-300"}>
              {existing.complete ? "complete" : "partial"}
            </span>
            {existing.matches > 1 && (
              <span className="text-gold-300">{existing.matches} matching folders</span>
            )}
          </div>
        </div>

        <div className="mt-4 flex flex-wrap items-center justify-end gap-2">
          <button type="button" onClick={onCancel} disabled={busy} className="btn-secondary">
            Cancel
          </button>
          <button type="button" onClick={onCopy} disabled={busy} className="btn-secondary">
            <Copy className="h-3 w-3" strokeWidth={1.9} />
            Keep both
          </button>
          <button type="button" onClick={onUpdate} disabled={busy} className="btn-primary">
            {busy ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
            ) : (
              <BookOpen className="h-3.5 w-3.5" strokeWidth={1.9} />
            )}
            Update existing
          </button>
        </div>
      </div>
    </div>
  );
}

function ChapterIndexPanel({
  index,
  loading,
  progress,
  error,
  onRetry,
  onRefresh,
}: {
  index: NovelChapterIndex | null;
  loading: boolean;
  /** Page counter for a paginated walk; `null` before the first page lands. */
  progress: { page: number; pages: number } | null;
  error: string | null;
  onRetry: () => void;
  onRefresh: () => void;
}) {
  return (
    <div className="mt-3 border border-parchment-100/8">
      <div className="flex items-center justify-between gap-3 border-b border-parchment-100/8 px-3 py-2">
        <div className="flex items-center gap-2">
          <BookOpen className="h-3 w-3 text-vermilion-300" strokeWidth={1.8} />
          <span className="text-[9.5px] font-semibold uppercase tracking-[0.16em] text-parchment-300">
            Chapter index
          </span>
        </div>
        {loading ? (
          <span className="flex items-center gap-1.5 font-mono text-[10px] tabular-nums text-parchment-500">
            <Loader2 className="h-2.5 w-2.5 animate-spin" strokeWidth={2} />
            {progress
              ? `TOC page ${progress.page} / ${progress.pages}`
              : "Reading every TOC page…"}
          </span>
        ) : index ? (
          <span className="flex items-center gap-2 font-mono text-[10px] tabular-nums text-parchment-500">
            <span className="flex items-center gap-1.5">
              {index.total.toLocaleString()} chapters · {index.source}
              {index.cached && (
                <span className="font-sans text-parchment-600">
                  · cached {formatCacheAge(index.cacheAgeSeconds)}
                </span>
              )}
              {!index.complete && (
                <span className="font-sans font-semibold uppercase tracking-[0.1em] text-gold-300">
                  incomplete
                </span>
              )}
            </span>
            <button
              type="button"
              onClick={onRefresh}
              title="Fetch the chapter index from the source instead of using the cache"
              className="font-sans text-[10px] uppercase tracking-[0.1em] text-parchment-500 transition-colors hover:text-parchment-200"
            >
              Refresh
            </button>
          </span>
        ) : (
          <span className="text-[10px] text-parchment-600">—</span>
        )}
      </div>

      {index?.titleSource && (
        <div className="border-b border-parchment-100/[0.045] px-3 py-1.5 text-[10px] text-parchment-600">
          Chapter names borrowed from{" "}
          <span className="font-mono text-parchment-400">
            {index.titleSource}
          </span>{" "}
          · this source leaves some chapters unnamed; names were matched by
          chapter number
        </div>
      )}

      {index && !index.complete && (
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-gold-400/15 bg-gold-400/[0.06] px-3 py-2.5">
          <span className="text-[11px] text-gold-200">
            {index.detail
              ? `Chapter index rejected: ${index.detail} The download will fail until the source list can be proven complete.`
              : "This chapter list is incomplete. The download will fail until every TOC page can be read."}
          </span>
          <button type="button" onClick={onRetry} className="btn-secondary h-7">
            Retry index
          </button>
        </div>
      )}

      {index?.transport && (
        <div className="border-b border-parchment-100/[0.045] px-3 py-1.5 font-mono text-[10px] text-parchment-600">
          transport · {index.transport}
        </div>
      )}

      {error && (
        <div className="flex flex-wrap items-center justify-between gap-2 px-3 py-2.5">
          <span className="text-[11px] text-parchment-500">
            Chapter index unavailable: {error}
          </span>
          <button type="button" onClick={onRetry} className="btn-secondary h-7">
            Retry index
          </button>
        </div>
      )}

      {loading && !index && (
        <div className="space-y-1.5 px-3 py-3">
          {[0, 1, 2, 3].map((row) => (
            <div
              key={row}
              className="h-2 animate-pulse-soft bg-parchment-100/[0.045]"
              style={{ width: `${88 - row * 11}%` }}
            />
          ))}
        </div>
      )}

      {index && index.chapters.length > 0 && (
        <div className="max-h-[196px] overflow-y-auto">
          {index.chapters.map((chapter) => (
            <div
              key={`${chapter.number}-${chapter.url}`}
              className="flex items-baseline gap-3 border-b border-parchment-100/[0.045] px-3 py-1.5 last:border-0"
            >
              <span className="w-10 shrink-0 text-right font-mono text-[10px] tabular-nums text-parchment-600">
                {chapter.number}
              </span>
              <span
                className="truncate text-[11.5px] text-parchment-300"
                title={chapter.title}
              >
                {chapter.title}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function formatCacheAge(seconds: number | null): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 60) {
    return "just now";
  }
  if (seconds < 3600) {
    return `${Math.floor(seconds / 60)}m ago`;
  }
  return `${Math.floor(seconds / 3600)}h ago`;
}

function withTimeout<T>(promise: Promise<T>, milliseconds: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(
      () => reject(new Error("The source took too long to respond.")),
      milliseconds,
    );
    promise.then(
      (value) => {
        window.clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        window.clearTimeout(timer);
        reject(error);
      },
    );
  });
}

function ResolvedPanel({
  match,
  meta,
  starting,
  chapters,
  onChaptersChange,
  throttle,
  onThrottleChange,
  onStart,
  onChooseAlternative,
  onClose,
}: {
  match: ResolvedTitle;
  meta: NovelMetadata | null;
  starting: boolean;
  chapters: string;
  onChaptersChange: (value: string) => void;
  throttle: boolean;
  onThrottleChange: (value: boolean) => void;
  onStart: () => void;
  onChooseAlternative: (result: NovelResult) => void;
  onClose: () => void;
}) {
  const author = meta?.author ?? meta?.authors[0] ?? null;
  const percent = confidencePercent(match.confidence);
  const tone = confidenceTone(match);
  const review = needsConfirmation(match);
  return (
    <section className="border border-vermilion-500/30 bg-[#0d0b0b]">
      <div className="flex items-start justify-between gap-3 border-b border-vermilion-500/15 px-4 py-2.5">
        <div className="flex items-center gap-2">
          <Sparkles className="h-3 w-3 text-vermilion-300" strokeWidth={1.9} />
          <span className="text-[9.5px] font-semibold uppercase tracking-[0.18em] text-vermilion-200">
            {review ? "Review resolved match" : "Resolved best match"}
          </span>
          <span
            className={cn(
              "border px-1.5 py-0.5 font-mono text-[9.5px] tracking-[0.08em]",
              tone === "review"
                ? "border-amber-500/40 text-amber-300"
                : "border-emerald-500/35 text-emerald-300",
            )}
            title={`Match confidence ${percent}% · ${matchTypeLabel(match.matchedBy)}`}
          >
            {percent}% {matchTypeLabel(match.matchedBy).toLowerCase()}
          </span>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Dismiss resolved match"
          className="flex h-5 w-5 items-center justify-center text-parchment-600 transition-colors hover:text-parchment-200"
        >
          <X className="h-3 w-3" strokeWidth={1.8} />
        </button>
      </div>
      <div className="px-4 py-3.5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex min-w-0 items-start gap-3">
            {meta?.cover ? (
              <img
                src={meta.cover}
                alt=""
                className="h-[92px] w-[66px] shrink-0 rounded-[2px] border border-parchment-100/14 object-cover"
              />
            ) : (
              <TitlePlate title={match.title} source={match.source} />
            )}
            <div className="min-w-0">
              <h3 className="font-display text-[23px] font-medium leading-none text-parchment-50">
                {meta?.title?.trim() || match.title}
              </h3>
              <div className="mt-1.5 flex flex-wrap items-center gap-2 text-[10px] uppercase tracking-[0.1em] text-parchment-500">
                <span>{match.detail}</span>
                {author && <span>· by {author}</span>}
                {meta?.chapterCount && (
                  <span>· {meta.chapterCount.toLocaleString()} chapters</span>
                )}
                {meta?.status && <span className="chip">{meta.status}</span>}
              </div>
              {meta?.synopsis && (
                <p className="mt-1.5 line-clamp-3 max-w-[620px] text-[11.5px] leading-relaxed text-parchment-400">
                  {meta.synopsis}
                </p>
              )}
              {meta && meta.tags.length > 0 && (
                <div className="mt-1.5 flex flex-wrap gap-1.5">
                  {meta.tags.slice(0, 8).map((tag) => (
                    <span
                      key={tag}
                      className="border border-parchment-100/10 px-1.5 py-0.5 text-[9px] uppercase tracking-[0.1em] text-parchment-500"
                    >
                      {tag}
                    </span>
                  ))}
                </div>
              )}
              <div className="mt-1 max-w-[560px] truncate font-mono text-[10px] text-parchment-600">
                {match.source}
              </div>
            </div>
          </div>
          <button
            type="button"
            onClick={onStart}
            disabled={starting}
            className="btn-primary h-9 shrink-0 px-4"
          >
            {starting ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
            ) : (
              <ArrowRight className="h-3.5 w-3.5" strokeWidth={1.9} />
            )}
            Start download
          </button>
        </div>
        {match.searched.length > 0 && (
          <p className="mt-2 text-[10px] leading-relaxed text-parchment-600">
            Searched {match.searched.length} source
            {match.searched.length === 1 ? "" : "s"}: {match.searched.join(" · ")}
            {match.cached && (
              <span
                className="ml-2 font-mono tabular-nums"
                title="Some search rows or detail pages came from the short-lived disk cache."
              >
                · cached {formatCacheAge(match.cacheAgeSeconds)}
              </span>
            )}
          </p>
        )}
        {review && (
          <div className="mt-2.5 border border-amber-500/30 bg-amber-500/[0.06] px-3 py-2">
            <p className="text-[10px] font-semibold uppercase tracking-[0.14em] text-amber-300">
              Confirm before downloading
            </p>
            <p className="mt-1 text-[11px] leading-relaxed text-amber-100/80">
              The resolver is only {percent}% sure this is “{match.input}”. An
              automatic title download will refuse it. Check the source, pick a
              different match below, or paste the page URL.
            </p>
          </div>
        )}
        {match.alternatives.length > 0 && (
          <div className="mt-2.5">
            <p className="text-[9.5px] font-semibold uppercase tracking-[0.14em] text-parchment-500">
              Other source matches
            </p>
            <div className="mt-1.5 flex flex-wrap gap-1.5">
              {match.alternatives.map((alternative) => (
                <button
                  key={alternative.url}
                  type="button"
                  onClick={() => onChooseAlternative(alternative)}
                  title={alternative.url}
                  className="border border-parchment-100/12 px-2 py-1 text-left text-[11px] text-parchment-300 transition-colors hover:border-vermilion-500/45 hover:bg-vermilion-500/[0.06] hover:text-parchment-50"
                >
                  <span className="block max-w-[220px] truncate">{alternative.title}</span>
                  <span className="block font-mono text-[9px] uppercase tracking-[0.08em] text-parchment-600">
                    {alternative.source}
                  </span>
                </button>
              ))}
            </div>
          </div>
        )}
        <CrawlOptions
          className="mt-3.5"
          chapters={chapters}
          onChaptersChange={onChaptersChange}
          throttle={throttle}
          onThrottleChange={onThrottleChange}
          throttleMode="rate-limit"
        />
      </div>
    </section>
  );
}

function SearchIntro({
  onExample,
  onResolve,
}: {
  onExample: (value: string) => void;
  onResolve: () => void;
}) {
  return (
    <section className="panel overflow-hidden">
      <div className="grid gap-0 md:grid-cols-[minmax(0,1.35fr)_minmax(0,1fr)]">
        <div className="p-5">
          <div className="section-label">The index</div>
          <h2 className="mt-2 font-display text-[28px] font-medium leading-[1.05] text-parchment-50">
            Type a title. We find the page.
          </h2>
          <p className="mt-2 max-w-[430px] text-[12.5px] leading-relaxed text-parchment-400">
            The selected sources are searched at once. Author mode reads each
            site&apos;s author page, so every novel credited to that writer is
            listed; sources without one fall back to page verification. Title
            mode also reads advertised aliases. When the site index comes up
            empty, the resolver checks AniList and Wikipedia for a canonical
            title or synonym, then searches again.
          </p>
          <ol className="mt-4 space-y-2">
            {[
              ["01", "Search by title, alias, author, or paste a source URL."],
              ["02", "Review the match confidence and pick a mirror."],
              ["03", "Choose every chapter, a range, or an exact list."],
            ].map(([index, text]) => (
              <li key={index} className="flex items-start gap-2.5">
                <span className="font-mono text-[10px] text-vermilion-400/80">{index}</span>
                <span className="text-[12px] leading-relaxed text-parchment-300">{text}</span>
              </li>
            ))}
          </ol>
        </div>
        <div className="border-t border-parchment-100/8 p-5 md:border-l md:border-t-0">
          <div className="section-label">Resolver examples</div>
          <div className="mt-3 flex flex-wrap gap-1.5">
            {EXAMPLES.map((example) => (
              <button
                key={example}
                type="button"
                onClick={() => onExample(example)}
                className="border border-parchment-100/12 px-2.5 py-1.5 text-[11px] text-parchment-300 transition-colors hover:border-vermilion-500/45 hover:bg-vermilion-500/[0.06] hover:text-parchment-50"
              >
                {example}
              </button>
            ))}
          </div>
          <p className="mt-3 text-[11px] leading-relaxed text-parchment-600">
            “Master of Gu” is an alias the source advertises, so the resolver can
            turn it into a full source link without pasting a URL. Prefix a query
            with “author:” (or switch to Author) to search by writer instead.
          </p>
          <button
            type="button"
            onClick={onResolve}
            className="mt-3 flex h-8 w-full items-center justify-center gap-1.5 border border-vermilion-500/30 text-[10.5px] font-semibold uppercase tracking-[0.12em] text-vermilion-300 transition-colors hover:bg-vermilion-500/[0.07] hover:text-vermilion-200"
          >
            <Sparkles className="h-3 w-3" strokeWidth={1.9} />
            Resolve current query
          </button>
        </div>
      </div>
    </section>
  );
}

function SearchSkeleton() {
  return (
    <section className="panel overflow-hidden">
      {[0, 1, 2].map((row) => (
        <div
          key={row}
          className="flex items-center gap-3 border-b border-parchment-100/[0.06] px-4 py-4 last:border-b-0"
        >
          <div className="h-[66px] w-[48px] shrink-0 animate-pulse-soft bg-parchment-100/[0.04]" />
          <div className="min-w-0 flex-1 space-y-2">
            <div className="h-3 w-1/3 animate-pulse-soft bg-parchment-100/[0.06]" />
            <div className="h-2 w-1/2 animate-pulse-soft bg-parchment-100/[0.035]" />
            <div className="h-1.5 w-2/3 animate-pulse-soft bg-parchment-100/[0.025]" />
          </div>
          <div className="h-8 w-20 animate-pulse-soft border border-parchment-100/8" />
        </div>
      ))}
    </section>
  );
}
