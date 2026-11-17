import { useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  ArrowRight,
  BookMarked,
  Check,
  ChevronRight,
  Database,
  Download,
  FolderOpen,
  Globe2,
  Layers,
  Loader2,
  Search,
  ShieldCheck,
} from "lucide-react";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import type {
  BookCandidate,
  BookSearchResponse,
  BookSource,
  HardcoverStatus,
} from "@/types";
import { api, isTauriRuntime } from "@/lib/tauri";
import { cn, formatBytes, formatDate } from "@/lib/utils";
import { PageHeader } from "@/components/ui/PageHeader";

const SOURCES: Array<{
  id: BookSource;
  name: string;
  tag: string;
  detail: string;
}> = [
  {
    id: "auto",
    name: "Auto order",
    tag: "Recommended",
    detail:
      "LibGen → WeLib → Anna’s Archive → OceanOfPDF. Every file record is scored against the selected title and author.",
  },
  {
    id: "libgen",
    name: "LibGen",
    tag: "Free · direct",
    detail:
      "HTML search plus a signed get.php link. No account, no browser. Measured 2026-11-16: 9–18 s per verified EPUB; the search adds the author so exact editions beat same-title variations.",
  },
  {
    id: "welib",
    name: "WeLib",
    tag: "Direct HTTP",
    detail:
      "Reads both welib mirrors without a browser. Measured 2026-11-16: one mirror served a CAPTCHA and the other returned HTTP 403, both inside 3 s, then the source was skipped.",
  },
  {
    id: "annas",
    name: "Anna’s Archive",
    tag: "Headless · slow",
    detail:
      "Free slow path behind DDoS-Guard. Measured 2026-11-16: 39–80 s per verified EPUB across the five-title run. Each source has a 150 s budget, so a stuck challenge becomes one visible skip line instead of a hung job.",
  },
  {
    id: "ocean",
    name: "OceanOfPDF",
    tag: "Last resort",
    detail:
      "Explicit only, never started by Auto. Measured 2026-11-16: headless Chromium did not clear Turnstile within 150 s, so it reports a challenge instead of opening a visible window. The opt-in NOVELFORGE_OCEAN_HEADFUL=1 debug path is not used by the app.",
  },
  {
    id: "gutenberg",
    name: "Project Gutenberg",
    tag: "Public domain · opt-in",
    detail:
      "Lawful catalogue of public-domain and older works. Searches Gutenberg’s own index, reads the book page to confirm the language, then downloads the EPUB3. Never part of Auto.",
  },
  {
    id: "standardebooks",
    name: "Standard Ebooks",
    tag: "Public domain · opt-in",
    detail:
      "Curated, professionally typeset public-domain EPUBs. English-only and smaller than Gutenberg’s catalogue. Never part of Auto.",
  },
];

const PUBLIC_DOMAIN_SOURCES = new Set<BookSource>(["gutenberg", "standardebooks"]);

const BOOK_KINDS = new Set(["epub", "pdf", "mobi", "azw3", "txt"]);

const EXAMPLES = ["The Empyrean", "The Lord of the Rings", "Dune"];

type SearchMode = "auto" | "books" | "series";

function normalizeTitle(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim();
}

function formatPosition(position: number | null): string {
  if (position === null || Number.isNaN(position)) return "—";
  return Number.isInteger(position) ? `#${position}` : `#${position.toFixed(1)}`;
}

function authorLine(book: BookCandidate): string {
  if (book.authors.length === 0) return "Unknown author";
  return book.authors.join(", ");
}

/**
 * A book counts as already shelved only when a *complete* library entry has
 * the same title and a compatible author. Substring matching would treat
 * “Dune” as present because “Dune Messiah” exists.
 */
function shelvesBook(
  novels: Array<{ title: string; author: string | null; complete: boolean }>,
  book: BookCandidate,
): boolean {
  const wanted = normalizeTitle(book.title);
  if (!wanted) return false;
  const authors = book.authors.map(normalizeTitle).filter(Boolean);
  return novels.some((novel) => {
    if (!novel.complete || normalizeTitle(novel.title) !== wanted) return false;
    if (authors.length === 0) return true;
    const shelfAuthor = normalizeTitle(novel.author ?? "");
    if (!shelfAuthor) return true;
    return authors.some(
      (author) =>
        author === shelfAuthor ||
        author.includes(shelfAuthor) ||
        shelfAuthor.includes(author),
    );
  });
}

export function BooksPage({
  manager,
  onOpenSettings,
}: {
  manager: DownloadManager;
  onOpenSettings: () => void;
}) {
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<SearchMode>("auto");
  const [source, setSource] = useState<BookSource>("auto");
  const [results, setResults] = useState<BookSearchResponse | null>(null);
  const [searching, setSearching] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [queued, setQueued] = useState<Set<string>>(new Set());
  const [hardcover, setHardcover] = useState<HardcoverStatus | null>(null);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    api
      .getHardcoverStatus()
      .then((status) => {
        if (!disposed) setHardcover(status);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
    };
  }, []);

  const books = useMemo(
    () =>
      manager.library.entries
        .filter((entry) => BOOK_KINDS.has(entry.kind))
        .slice(0, 6),
    [manager.library.entries],
  );

  const language = manager.settings?.bookLanguage;

  function isShelved(book: BookCandidate): boolean {
    return shelvesBook(manager.library.novels ?? [], book);
  }

  const selectedSeriesBooks = useMemo(() => {
    if (!results?.series) return [];
    return results.series.books.filter(
      (book) => book.released && selected.has(book.id),
    );
  }, [results, selected]);

  async function runSearch() {
    const trimmed = query.trim();
    if (trimmed.length < 2) {
      manager.showNotice(
        "error",
        "Search needs a title",
        "Type at least two characters.",
      );
      return;
    }
    setSearching(true);
    setQueued(new Set());
    try {
      const response = await api.searchBooks(trimmed, mode);
      setResults(response);
      const released = response.series?.books.filter((book) => book.released) ?? [];
      setSelected(new Set(released.map((book) => book.id)));
    } catch (error) {
      setResults(null);
      manager.showNotice(
        "error",
        "Catalogue search failed",
        error instanceof Error ? error.message : String(error),
      );
    } finally {
      setSearching(false);
    }
  }

  async function queueBook(book: BookCandidate): Promise<boolean> {
    if (isShelved(book)) {
      manager.showNotice(
        "success",
        "Already in library",
        `${book.title} is already complete on your shelf, so it was skipped. Delete it in Library to download it again.`,
      );
      return false;
    }
    const started = await manager.start(
      "books",
      book.title,
      {
        bookSource: source,
        bookTitle: book.title,
        bookAuthor: book.authors.length > 0 ? book.authors.join(", ") : null,
        bookIsbn: book.isbn,
        bookSeries: book.series ?? results?.series?.name ?? null,
        bookSeriesPosition: book.seriesPosition,
      },
      book.title,
    );
    if (started) {
      setQueued((current) => new Set(current).add(book.id));
    }
    return started;
  }

  async function queueSeries() {
    if (selectedSeriesBooks.length === 0) {
      manager.showNotice(
        "error",
        "No books selected",
        "Select at least one released book from the series.",
      );
      return;
    }
    const pending = selectedSeriesBooks.filter((book) => !isShelved(book));
    const skipped = selectedSeriesBooks.length - pending.length;
    if (pending.length === 0) {
      manager.showNotice(
        "success",
        "Series already complete",
        `All ${skipped} selected book(s) are already complete on your shelf. Delete a book in Library to download it again.`,
      );
      return;
    }
    let started = 0;
    for (const book of pending) {
      if (await queueBook(book)) started += 1;
    }
    manager.showNotice(
      started > 0 ? "success" : "error",
      "Series queue",
      started > 0
        ? `${started} of ${pending.length} book(s) queued${skipped > 0 ? ` · ${skipped} already complete and skipped` : ""}. Each download is title/author verified and shelved as Author/Series/NN - Title.`
        : "No book could be queued. Check Guardrails for the aget toolchain.",
    );
  }

  function toggleSelected(id: string) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  const failures = results?.failures ?? [];

  return (
    <div className="space-y-4">
      <PageHeader
        eyebrow="Collection 03 / Books"
        title="EPUB Books"
        description="Search a real catalogue first, then download from free sources. Series are resolved in reading order and every file record is checked against the selected title, author and ISBN."
        actions={
          <>
            <span className="chip">LibGen</span>
            <span className="chip">WeLib</span>
            <span className="chip">Anna’s Archive</span>
            <span className="chip chip-vermilion">EPUB</span>
          </>
        }
      />

      <div className="grid grid-cols-1 gap-3.5 lg:grid-cols-[minmax(0,1fr)_310px]">
        <section className="panel overflow-hidden">
          <div className="flex items-center justify-between border-b border-parchment-100/8 px-4 py-2.5">
            <div className="flex items-center gap-2">
              <Search className="h-3 w-3 text-vermilion-400" strokeWidth={1.9} />
              <span className="section-label">Catalogue search</span>
            </div>
            <span className="text-[10px] text-parchment-600">
              {hardcover?.configured ? "Hardcover + Open Library" : "Open Library"}
            </span>
          </div>

          <form
            onSubmit={(event) => {
              event.preventDefault();
              void runSearch();
            }}
            className="p-4"
          >
            <div className="flex flex-col gap-2 sm:flex-row">
              <div className="field h-[50px] min-w-0 flex-1">
                <BookMarked className="h-4 w-4 shrink-0 text-parchment-500" strokeWidth={1.7} />
                <input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="Series or book — The Empyrean, The Lord of the Rings, Dune…"
                  aria-label="Book or series title"
                  spellCheck={false}
                  autoComplete="off"
                  className="text-[14.5px]"
                />
              </div>
              <button
                type="submit"
                disabled={searching}
                className="btn-primary h-[50px] w-full shrink-0 gap-2 px-5 sm:w-auto"
              >
                {searching ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={2} />
                ) : (
                  <ArrowRight className="h-3.5 w-3.5" strokeWidth={1.9} />
                )}
                Search catalogue
              </button>
            </div>

            <div className="mt-2 flex flex-wrap items-center gap-2">
              <span className="text-[10px] font-semibold uppercase tracking-[0.14em] text-parchment-600">
                Try
              </span>
              {EXAMPLES.map((example) => (
                <button
                  key={example}
                  type="button"
                  onClick={() => setQuery(example)}
                  className="border border-parchment-100/10 px-2 py-1 text-[11px] text-parchment-400 transition-colors hover:border-vermilion-500/45 hover:bg-vermilion-500/[0.06] hover:text-parchment-100"
                >
                  {example}
                </button>
              ))}
              <div className="ml-auto flex items-center gap-1">
                {(["auto", "books", "series"] as SearchMode[]).map((value) => (
                  <button
                    key={value}
                    type="button"
                    onClick={() => setMode(value)}
                    aria-pressed={mode === value}
                    className={cn(
                      "border px-2 py-1 text-[10px] font-semibold uppercase tracking-[0.1em] transition-colors",
                      mode === value
                        ? "border-vermilion-500/50 bg-vermilion-900/25 text-vermilion-100"
                        : "border-parchment-100/10 text-parchment-600 hover:text-parchment-300",
                    )}
                  >
                    {value}
                  </button>
                ))}
              </div>
            </div>

            <div className="mt-4 border-t border-parchment-100/7 pt-3.5">
              <div className="mb-2 flex items-center justify-between">
                <span className="text-[9.5px] font-semibold uppercase tracking-[0.16em] text-parchment-500">
                  Download source
                </span>
                <span className="text-[10px] text-parchment-600">
                  Auto order: LibGen → WeLib → Anna’s → Ocean
                </span>
              </div>

              <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                {SOURCES.map((option) => {
                  const active = source === option.id;
                  return (
                    <button
                      key={option.id}
                      type="button"
                      onClick={() => setSource(option.id)}
                      aria-pressed={active}
                      className={cn(
                        "relative border p-3 text-left transition-colors",
                        active
                          ? "border-vermilion-500/55 bg-vermilion-900/20"
                          : "border-parchment-100/9 bg-transparent hover:border-parchment-100/20 hover:bg-parchment-100/[0.025]",
                      )}
                    >
                      <div className="flex items-start justify-between gap-2">
                        <span
                          className={cn(
                            "text-[13px] font-semibold",
                            active ? "text-parchment-50" : "text-parchment-200",
                          )}
                        >
                          {option.name}
                        </span>
                        <span
                          className={cn(
                            "shrink-0 border px-1 py-[1px] text-[9px] font-semibold uppercase tracking-[0.08em]",
                            active
                              ? "border-vermilion-500/40 text-vermilion-200"
                              : "border-parchment-100/10 text-parchment-600",
                          )}
                        >
                          {option.tag}
                        </span>
                      </div>
                      <p className="mt-1.5 text-[10.5px] leading-relaxed text-parchment-500">
                        {option.detail}
                      </p>
                      <span
                        className={cn(
                          "absolute bottom-0 left-0 h-[2px] transition-[width]",
                          active ? "w-full bg-vermilion-500" : "w-0 bg-transparent",
                        )}
                      />
                    </button>
                  );
                })}
              </div>

              {PUBLIC_DOMAIN_SOURCES.has(source) && (
                <p className="mt-2.5 border-l-2 border-vermilion-500/35 pl-2.5 text-[10.5px] leading-relaxed text-parchment-500">
                  Opt-in public-domain catalogue. It never joins Auto and only
                  covers older, freely licensed works — search the catalogue
                  again after switching, then Fetch the title you want.
                </p>
              )}
            </div>
          </form>
        </section>

        <section className="panel flex flex-col p-4">
          <div className="flex items-center gap-2">
            <Database className="h-3 w-3 text-vermilion-400" strokeWidth={1.9} />
            <span className="section-label">Metadata</span>
          </div>

          <div className="mt-3 space-y-2.5">
            <div className="flex items-start gap-2.5">
              <span className="mt-[1px] font-mono text-[10px] text-vermilion-400/75">01</span>
              <div className="min-w-0">
                <div className="text-[12px] font-medium text-parchment-200">
                  Hardcover {hardcover?.configured ? "· local token" : "· not configured"}
                </div>
                <p className="mt-0.5 text-[10.5px] leading-relaxed text-parchment-600">
                  Series positions, authors and ISBNs. The token stays in a local
                  file and is sent only to api.hardcover.app.
                </p>
              </div>
            </div>
            <div className="flex items-start gap-2.5">
              <span className="mt-[1px] font-mono text-[10px] text-vermilion-400/75">02</span>
              <div className="min-w-0">
                <div className="text-[12px] font-medium text-parchment-200">
                  Open Library
                </div>
                <p className="mt-0.5 text-[10.5px] leading-relaxed text-parchment-600">
                  No token, no account. Series search uses
                  {" "}
                  <code className="text-parchment-400">series:“name”</code> and falls
                  back to title matches.
                </p>
              </div>
            </div>
          </div>

          <button
            type="button"
            onClick={onOpenSettings}
            className="mt-3 flex items-center justify-between border border-parchment-100/9 bg-parchment-100/[0.018] px-2.5 py-2 text-left transition-colors hover:border-vermilion-500/35"
          >
            <span className="text-[11px] text-parchment-300">
              {hardcover?.configured ? "Manage Hardcover token" : "Add a Hardcover token"}
            </span>
            <ChevronRight className="h-3 w-3 text-parchment-600" strokeWidth={1.8} />
          </button>

          <div className="mt-3 border-t border-parchment-100/7 pt-3">
            <div className="flex items-start gap-2 border border-gold-400/20 bg-gold-400/[0.045] px-2.5 py-2">
              <ShieldCheck className="mt-[1px] h-3 w-3 shrink-0 text-gold-300" strokeWidth={1.8} />
              <p className="text-[10.5px] leading-relaxed text-parchment-400">
                Downloads fail closed. If no file record matches the selected
                title and author, nothing is shelved.
              </p>
            </div>
          </div>

          <div className="mt-auto flex items-center justify-between gap-2 border-t border-parchment-100/7 pt-3">
            <span className="text-[10px] uppercase tracking-[0.12em] text-parchment-600">
              Language
            </span>
            <span className="chip">{language || "any"}</span>
          </div>
        </section>
      </div>

      {results && (
        <section className="panel overflow-hidden">
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-parchment-100/8 px-4 py-2.5">
            <div className="flex items-center gap-2">
              <Layers className="h-3 w-3 text-vermilion-400" strokeWidth={1.9} />
              <span className="section-label">Catalogue results</span>
            </div>
            <div className="flex flex-wrap items-center gap-2 text-[10px] text-parchment-600">
              <span>{results.searched.join(" · ") || "no source answered"}</span>
              {results.cached && (
                <span className="border border-parchment-100/10 px-1.5 py-[1px] uppercase tracking-[0.08em]">
                  cached{results.cacheAgeSeconds ? ` · ${results.cacheAgeSeconds}s` : ""}
                </span>
              )}
            </div>
          </div>

          {failures.length > 0 && (
            <div className="border-b border-parchment-100/7 px-4 py-2.5">
              {failures.map((failure) => (
                <div
                  key={`${failure.source}-${failure.reason}`}
                  className="flex items-start gap-2 text-[11px] text-gold-200"
                >
                  <AlertTriangle className="mt-[1px] h-3 w-3 shrink-0" strokeWidth={1.8} />
                  <span>
                    <span className="font-semibold">{failure.label}:</span>{" "}
                    {failure.reason}
                  </span>
                </div>
              ))}
            </div>
          )}

          {results.series && (
            <div className="border-b border-parchment-100/8">
              <div className="flex flex-wrap items-center justify-between gap-2 bg-parchment-100/[0.02] px-4 py-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="chip chip-vermilion">Series</span>
                    <span className="truncate text-[14px] font-semibold text-parchment-100">
                      {results.series.name}
                    </span>
                  </div>
                  <p className="mt-1 text-[10.5px] text-parchment-600">
                    {results.series.author || "Unknown author"} ·{" "}
                    {results.series.books.length} catalogue book(s) ·{" "}
                    {results.series.source}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => void queueSeries()}
                  disabled={manager.starting || selectedSeriesBooks.length === 0}
                  className="btn-primary h-9 shrink-0 gap-2 px-4"
                >
                  <Download className="h-3.5 w-3.5" strokeWidth={1.9} />
                  Queue {selectedSeriesBooks.length} selected
                </button>
              </div>

              <div>
                {results.series.books.map((book) => {
                  const checked = selected.has(book.id);
                  const done = queued.has(book.id);
                  const shelved = isShelved(book);
                  return (
                    <div
                      key={book.id}
                      className={cn(
                        "flex items-center gap-3 border-b border-parchment-100/[0.055] px-4 py-2.5 last:border-0",
                        !book.released && "opacity-55",
                      )}
                    >
                      <button
                        type="button"
                        onClick={() => book.released && toggleSelected(book.id)}
                        disabled={!book.released || done}
                        aria-pressed={checked}
                        aria-label={`Select ${book.title}`}
                        className={cn(
                          "flex h-5 w-5 shrink-0 items-center justify-center border transition-colors",
                          checked || done
                            ? "border-vermilion-500/60 bg-vermilion-500/20 text-vermilion-100"
                            : "border-parchment-100/20 text-transparent hover:border-parchment-100/40",
                        )}
                      >
                        {(checked || done) && <Check className="h-3 w-3" strokeWidth={2.4} />}
                      </button>

                      <span className="w-11 shrink-0 font-mono text-[10px] text-vermilion-300/85">
                        {formatPosition(book.seriesPosition)}
                      </span>

                      {book.coverUrl ? (
                        <img
                          src={book.coverUrl}
                          alt=""
                          loading="lazy"
                          className="h-11 w-8 shrink-0 border border-parchment-100/10 object-cover"
                          onError={(event) => {
                            event.currentTarget.style.visibility = "hidden";
                          }}
                        />
                      ) : (
                        <span className="flex h-11 w-8 shrink-0 items-center justify-center border border-parchment-100/10 bg-[#0a0a0c]">
                          <BookMarked className="h-3 w-3 text-parchment-600" strokeWidth={1.6} />
                        </span>
                      )}

                      <div className="min-w-0 flex-1">
                        <div className="truncate text-[13px] font-medium text-parchment-200">
                          {book.title}
                        </div>
                        <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-parchment-600">
                          <span>{authorLine(book)}</span>
                          {book.year && <span>· {book.year}</span>}
                          {book.isbn && <span className="font-mono">· {book.isbn}</span>}
                          {!book.released && (
                            <span className="text-gold-300">· upcoming</span>
                          )}
                          {shelved && <span className="text-jade-400">· already shelved</span>}
                        </div>
                      </div>

                      {done ? (
                        <span className="flex items-center gap-1 text-[10px] font-semibold uppercase tracking-[0.1em] text-jade-300">
                          <Check className="h-3 w-3" strokeWidth={2.2} />
                          queued
                        </span>
                      ) : shelved ? (
                        <span className="flex items-center gap-1 text-[10px] font-semibold uppercase tracking-[0.1em] text-jade-300">
                          <Check className="h-3 w-3" strokeWidth={2.2} />
                          shelved
                        </span>
                      ) : (
                        <button
                          type="button"
                          onClick={() => void queueBook(book)}
                          disabled={!book.released || manager.starting}
                          className="shrink-0 border border-parchment-100/12 px-2.5 py-1 text-[10px] font-semibold uppercase tracking-[0.1em] text-parchment-300 transition-colors hover:border-vermilion-500/45 hover:text-vermilion-100 disabled:cursor-not-allowed disabled:opacity-40"
                        >
                          Download
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          <div>
            <div className="flex items-center justify-between bg-parchment-100/[0.012] px-4 py-2">
              <span className="text-[9.5px] font-semibold uppercase tracking-[0.16em] text-parchment-500">
                {results.series ? "Other book matches" : "Book matches"}
              </span>
              <span className="text-[10px] text-parchment-600">
                {results.candidates.length} candidate(s)
              </span>
            </div>

            {results.candidates.length === 0 ? (
              <div className="flex flex-col items-center justify-center px-6 py-8 text-center">
                <Globe2 className="h-5 w-5 text-parchment-600" strokeWidth={1.5} />
                <div className="mt-2.5 text-[13px] font-medium text-parchment-300">
                  {results.series ? "No extra book rows" : "No catalogue match"}
                </div>
                <p className="mt-1 max-w-[380px] text-[11.5px] leading-relaxed text-parchment-600">
                  Try the author, a shorter title, or switch the search mode.
                  Failed sources are listed above; nothing is downloaded until a
                  verified record is selected.
                </p>
              </div>
            ) : (
              results.candidates.map((book) => {
                const done = queued.has(book.id);
                const shelved = isShelved(book);
                return (
                  <div
                    key={book.id}
                    className="flex items-center gap-3 border-b border-parchment-100/[0.055] px-4 py-2.5 last:border-0"
                  >
                    {book.coverUrl ? (
                      <img
                        src={book.coverUrl}
                        alt=""
                        loading="lazy"
                        className="h-12 w-9 shrink-0 border border-parchment-100/10 object-cover"
                        onError={(event) => {
                          event.currentTarget.style.visibility = "hidden";
                        }}
                      />
                    ) : (
                      <span className="flex h-12 w-9 shrink-0 items-center justify-center border border-parchment-100/10 bg-[#0a0a0c]">
                        <BookMarked className="h-3 w-3 text-parchment-600" strokeWidth={1.6} />
                      </span>
                    )}

                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-[13px] font-medium text-parchment-200">
                          {book.title}
                        </span>
                        {book.series && (
                          <span className="shrink-0 border border-parchment-100/10 px-1 py-[1px] text-[9px] uppercase tracking-[0.08em] text-parchment-600">
                            {book.series} {formatPosition(book.seriesPosition)}
                          </span>
                        )}
                      </div>
                      <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-parchment-600">
                        <span>{authorLine(book)}</span>
                        {book.year && <span>· {book.year}</span>}
                        {book.publisher && <span>· {book.publisher}</span>}
                        {book.isbn && <span className="font-mono">· {book.isbn}</span>}
                        {shelved && <span className="text-jade-400">· already shelved</span>}
                      </div>
                    </div>

                    {done ? (
                      <span className="flex items-center gap-1 text-[10px] font-semibold uppercase tracking-[0.1em] text-jade-300">
                        <Check className="h-3 w-3" strokeWidth={2.2} />
                        queued
                      </span>
                    ) : shelved ? (
                      <span className="flex items-center gap-1 text-[10px] font-semibold uppercase tracking-[0.1em] text-jade-300">
                        <Check className="h-3 w-3" strokeWidth={2.2} />
                        shelved
                      </span>
                    ) : (
                      <button
                        type="button"
                        onClick={() => void queueBook(book)}
                        disabled={manager.starting}
                        className="btn-secondary h-8 shrink-0 gap-1.5 px-3 text-[10px]"
                      >
                        <Download className="h-3 w-3" strokeWidth={1.9} />
                        Fetch
                      </button>
                    )}
                  </div>
                );
              })
            )}
          </div>
        </section>
      )}

      <section className="panel overflow-hidden">
        <div className="flex items-center justify-between border-b border-parchment-100/8 px-4 py-2.5">
          <div className="flex items-center gap-2">
            <BookMarked className="h-3 w-3 text-vermilion-400" strokeWidth={1.9} />
            <span className="section-label">EPUB shelf</span>
          </div>
          <button
            type="button"
            onClick={() => void manager.openDownloads()}
            className="flex items-center gap-1.5 text-[10.5px] font-semibold uppercase tracking-[0.12em] text-parchment-400 transition-colors hover:text-vermilion-200"
          >
            <FolderOpen className="h-3 w-3" strokeWidth={1.8} />
            Open folder
          </button>
        </div>

        {books.length === 0 ? (
          <div className="flex flex-col items-center justify-center px-6 py-9 text-center">
            <BookMarked className="h-5 w-5 text-parchment-600" strokeWidth={1.5} />
            <div className="mt-2.5 text-[13px] font-medium text-parchment-300">
              No books shelved yet
            </div>
            <p className="mt-1 max-w-[340px] text-[11.5px] leading-relaxed text-parchment-600">
              Search for a series or a title above, pick the exact record, and
              queue it. Verified EPUBs appear here and in the library.
            </p>
          </div>
        ) : (
          <div>
            {books.map((entry) => (
              <button
                key={entry.path}
                type="button"
                onClick={() => void manager.reveal(entry.path)}
                title={entry.path}
                className="group flex w-full items-center gap-3 border-b border-parchment-100/[0.055] px-4 py-2.5 text-left last:border-0 transition-colors hover:bg-parchment-100/[0.025]"
              >
                <span className="flex h-8 w-7 shrink-0 items-center justify-center border border-parchment-100/12 bg-[#0a0a0c] font-mono text-[9px] font-semibold uppercase tracking-[0.06em] text-parchment-400">
                  {entry.kind}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] font-medium text-parchment-200 transition-colors group-hover:text-parchment-50">
                    {entry.name}
                  </span>
                  <span className="mt-0.5 block text-[10px] uppercase tracking-[0.1em] text-parchment-600">
                    {formatBytes(entry.size)} · {formatDate(entry.modified)}
                  </span>
                </span>
                <ChevronRight
                  className="h-3.5 w-3.5 shrink-0 text-parchment-600 transition-colors group-hover:text-vermilion-300"
                  strokeWidth={1.8}
                />
              </button>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
