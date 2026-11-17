import { hostFromUrl } from "@/lib/utils";
import { useMemo, useState } from "react";
import {
  ArrowDownUp,
  ArrowRight,
  ChevronDown,
  CornerDownLeft,
  Globe2,
  LayoutGrid,
  Loader2,
  Search,
  SlidersHorizontal,
  Sparkles,
} from "lucide-react";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import type { Page } from "@/types";
import { HeroBanner } from "@/components/home/HeroBanner";
import { ShelfRow, type ShelfItem } from "@/components/novels/ShelfRow";

interface Feature extends ShelfItem {
  sourceKey: string;
  modified: number;
  chapterTotal: number;
}

type SortKey = "recent" | "chapters" | "title";

export function HomePage({
  manager,
  onNavigate,
  onSearch,
}: {
  manager: DownloadManager;
  enginesReady: number;
  enginesTotal: number;
  onNavigate: (page: Page) => void;
  onSearch: (query: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [site, setSite] = useState("all");
  const [genre, setGenre] = useState("All Genres");
  const [sort, setSort] = useState<SortKey>("recent");
  const [advanced, setAdvanced] = useState(false);

  const novels = (manager.library.novels ?? []).filter(
    (novel) => novel.complete !== false,
  );

  const shelf = useMemo<Feature[]>(
    () =>
      novels.map((novel) => {
        const source = novel.sourceUrl
          ? hostFromUrl(novel.sourceUrl)
          : "local";
        const tags =
          novel.tags.length > 0
            ? novel.tags.slice(0, 4)
            : [novel.author?.trim() || "Downloaded"];
        return {
          id: novel.folder,
          title: novel.title,
          tags,
          description:
            novel.synopsis?.trim() ||
            `${novel.downloadedChapters.toLocaleString()} of ${novel.chapterCount.toLocaleString()} chapters downloaded${
              novel.author ? ` · ${novel.author}` : ""
            }`,
          chapters: `${novel.downloadedChapters.toLocaleString()} / ${novel.chapterCount.toLocaleString()}`,
          source,
          cover: novel.cover ?? undefined,
          coverSeed: novel.folder,
          actionLabel: "Open",
          sourceKey: source,
          modified: novel.modified,
          chapterTotal: novel.chapterCount,
          onSelect: () => void manager.reveal(novel.folder),
        };
      }),
    [manager, novels],
  );

  const siteOptions = useMemo(() => {
    const counts = new Map<string, number>();
    for (const item of shelf) {
      counts.set(item.sourceKey, (counts.get(item.sourceKey) ?? 0) + 1);
    }
    const downloaded = Array.from(counts.entries())
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([source, count]) => {
        const label = source === "local" ? "Local / imported" : source;
        return [source, `${label} · ${count} ${count === 1 ? "novel" : "novels"}`] as [
          string,
          string,
        ];
      });
    return [
      [
        "all",
        `All downloaded sources · ${shelf.length} ${
          shelf.length === 1 ? "novel" : "novels"
        }`,
      ] as [string, string],
      ...downloaded,
    ];
  }, [shelf]);

  // A source can disappear from the shelf (last novel deleted or filtered out).
  // Fall back to "all" instead of leaving the select blank and the shelf empty.
  const activeSite = siteOptions.some(([value]) => value === site) ? site : "all";

  const genreOptions = useMemo(
    () => [
      "All Genres",
      ...Array.from(new Set(shelf.flatMap((item) => item.tags))).sort(),
    ],
    [shelf],
  );

  const items = useMemo(() => {
    let next = shelf.slice();
    if (activeSite !== "all") next = next.filter((item) => item.sourceKey === activeSite);
    if (genre !== "All Genres") next = next.filter((item) => item.tags.includes(genre));
    if (sort === "title") {
      next.sort((left, right) => left.title.localeCompare(right.title));
    } else if (sort === "chapters") {
      next.sort((left, right) => right.chapterTotal - left.chapterTotal);
    } else {
      next.sort((left, right) => right.modified - left.modified);
    }
    return next;
  }, [activeSite, genre, shelf, sort]);

  function search(value: string) {
    const trimmed = value.trim();
    if (trimmed.length < 2) {
      manager.showNotice("error", "Search needs a title", "Type at least two characters.");
      return;
    }
    onSearch(trimmed);
  }

  const nothingArchived = !manager.loading && shelf.length === 0;

  return (
    <div className="pb-8">
      <div className="-mx-4 -mt-4 sm:-mx-6 xl:-mx-8 xl:-mt-6">
        <HeroBanner />
      </div>

      <div className="pt-4">
        <form
          onSubmit={(event) => {
            event.preventDefault();
            search(query);
          }}
          className="flex h-[46px] rounded-[5px] border border-[#393639] bg-[#111113] transition-colors focus-within:border-[#6f3a3e]"
        >
          <div className="flex min-w-0 flex-1 items-center gap-3 pl-4 pr-3">
            <Search className="h-[18px] w-[18px] shrink-0 text-[#aaa6a8]" strokeWidth={1.7} />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search for novels, or paste a novel URL..."
              aria-label="Search for novels or paste a novel URL"
              spellCheck={false}
              autoComplete="off"
              className="h-full w-full min-w-0 bg-transparent text-[16px] text-[#e8e3e4] outline-none placeholder:text-[#969295]"
            />
            {query && (
              <button
                type="button"
                onClick={() => setQuery("")}
                className="shrink-0 text-[15px] text-[#777276] transition-colors hover:text-[#d8d6d7]"
              >
                Clear
              </button>
            )}
          </div>
          <button
            type="submit"
            className="-m-px flex h-[46px] w-[131px] shrink-0 items-center justify-center gap-2 rounded-r-[5px] border border-[#8e3a40] bg-[#321b1e] text-[16px] text-[#d8d6d7] transition-colors hover:border-[#a94349] hover:bg-[#3a1e21] hover:text-[#f0e9e9]"
          >
            Search
            <CornerDownLeft className="h-[15px] w-[15px]" strokeWidth={1.6} />
          </button>
        </form>

        <div className="mt-[15px] flex flex-wrap gap-[17px]">
          <FilterSelect
            className="flex-[1_1_265px]"
            icon={<Globe2 className="h-[15px] w-[15px]" strokeWidth={1.6} />}
            value={activeSite}
            onChange={setSite}
            options={siteOptions}
          />
          <FilterSelect
            className="flex-[1_1_190px]"
            icon={<LayoutGrid className="h-[15px] w-[15px]" strokeWidth={1.6} />}
            value={genre}
            onChange={setGenre}
            options={genreOptions.map((value) => [value, value] as [string, string])}
          />
          <FilterSelect
            className="flex-[1_1_205px]"
            icon={<ArrowDownUp className="h-[15px] w-[15px]" strokeWidth={1.6} />}
            value={sort}
            onChange={(value) => setSort(value as SortKey)}
            options={[
              ["recent", "Recently Added"],
              ["chapters", "Most Chapters"],
              ["title", "Title A–Z"],
            ]}
          />
          <button
            type="button"
            onClick={() => setAdvanced((current) => !current)}
            aria-expanded={advanced}
            className="flex h-[41px] flex-[1_1_300px] items-center gap-3 rounded-[5px] border border-[#302d30] bg-[#101012] px-[15px] text-left text-[16px] text-[#d8d6d7] transition-colors hover:border-[#454145]"
          >
            <span
              className={`flex h-[17px] w-[31px] shrink-0 items-center rounded-full px-[2px] transition-colors ${
                advanced ? "bg-[#8e353b]" : "bg-[#363438]"
              }`}
            >
              <span
                className={`h-[13px] w-[13px] rounded-full bg-[#c8c5c6] transition-transform ${
                  advanced ? "translate-x-[14px]" : "translate-x-0"
                }`}
              />
            </span>
            <span className="truncate">Advanced Filters</span>
            <ChevronDown
              className={`ml-auto h-[15px] w-[15px] shrink-0 text-[#969295] transition-transform ${
                advanced ? "rotate-180" : ""
              }`}
              strokeWidth={1.6}
            />
          </button>
        </div>

        {advanced && (
          <div className="mt-[15px] flex flex-wrap items-center gap-x-5 gap-y-3 rounded-[5px] border border-[#302d30] bg-[#0d0d0f] px-4 py-3">
            <div className="flex min-w-0 flex-1 items-center gap-2.5 text-[15px] text-[#969295]">
              <SlidersHorizontal className="h-4 w-4 shrink-0 text-[#c28b65]" strokeWidth={1.6} />
              Chapter ranges, source resolution, and crawler jobs are configured in the webnovel studio.
            </div>
            <button
              type="button"
              onClick={() => onNavigate("webnovels")}
              className="flex h-[32px] shrink-0 items-center gap-2 rounded-[4px] border border-[#3a373a] px-3.5 text-[15px] text-[#d8d6d7] transition-colors hover:border-[#8d3b40] hover:text-[#eee9e8]"
            >
              Open crawler
              <ArrowRight className="h-3.5 w-3.5" strokeWidth={1.6} />
            </button>
          </div>
        )}

        <section className="mt-4">
          {manager.loading ? (
            <div className="flex items-center gap-3 border-y border-[#2b282b] px-2 py-14 text-[#969295]">
              <Loader2 className="h-4 w-4 animate-spin" strokeWidth={1.7} />
              <span className="text-[15px]">Reading the local library…</span>
            </div>
          ) : nothingArchived ? (
            <div className="flex flex-col items-center justify-center border-y border-[#2b282b] px-6 py-14 text-center">
              <Sparkles className="h-5 w-5 text-[#5d595c]" strokeWidth={1.5} />
              <div className="mt-3 font-display text-[23px] text-[#d8d6d7]">
                No downloads yet
              </div>
              <p className="mt-1.5 max-w-[440px] text-[15px] leading-relaxed text-[#969295]">
                Search a title above, choose a source and chapter range, and the
                finished book will appear on this shelf with its real cover and metadata.
              </p>
              <button
                type="button"
                onClick={() => onNavigate("webnovels")}
                className="mt-4 flex h-[35px] items-center gap-2 rounded-[4px] border border-[#3a373a] bg-[#0d0d0f] px-4 text-[15px] text-[#d8d6d7] transition-colors hover:border-[#8d3b40] hover:text-[#eee9e8]"
              >
                Go to Webnovels
                <ArrowRight className="h-3.5 w-3.5" strokeWidth={1.6} />
              </button>
            </div>
          ) : items.length === 0 ? (
            <div className="flex flex-col items-center justify-center border-y border-[#2b282b] px-6 py-14 text-center">
              <Sparkles className="h-5 w-5 text-[#5d595c]" strokeWidth={1.5} />
              <div className="mt-3 font-display text-[23px] text-[#d8d6d7]">
                Nothing on this shelf
              </div>
              <p className="mt-1.5 max-w-[420px] text-[15px] leading-relaxed text-[#969295]">
                Adjust the filters, or search a title and let the resolver find its source page.
              </p>
              <button
                type="button"
                onClick={() => {
                  setSite("all");
                  setGenre("All Genres");
                }}
                className="mt-4 flex h-[35px] items-center gap-2 rounded-[4px] border border-[#3a373a] bg-[#0d0d0f] px-4 text-[15px] text-[#d8d6d7] transition-colors hover:border-[#8d3b40] hover:text-[#eee9e8]"
              >
                Reset filters
                <ArrowRight className="h-3.5 w-3.5" strokeWidth={1.6} />
              </button>
            </div>
          ) : (
            items.map((item, index) => (
              <ShelfRow
                key={item.id}
                index={index}
                item={item}
              />
            ))
          )}
        </section>
      </div>
    </div>
  );
}

function FilterSelect({
  icon,
  value,
  onChange,
  options,
  className,
}: {
  icon: React.ReactNode;
  value: string;
  onChange: (value: string) => void;
  options: Array<[string, string]>;
  className?: string;
}) {
  const selectedLabel =
    options.find(([optionValue]) => optionValue === value)?.[1] ?? value;
  return (
    <div
      className={`relative flex h-[41px] min-w-0 items-center rounded-[5px] border border-[#302d30] bg-[#101012] ${className ?? ""}`}
    >
      <span className="pointer-events-none absolute left-[14px] text-[#969295]">{icon}</span>
      <select
        value={value}
        onChange={(event) => onChange(event.target.value)}
        title={selectedLabel}
        aria-label={selectedLabel}
        className="h-full w-full appearance-none truncate bg-transparent pl-[41px] pr-8 text-[16px] text-[#d8d6d7] outline-none"
      >
        {options.map(([optionValue, label]) => (
          <option key={optionValue} value={optionValue} className="bg-[#101012]">
            {label}
          </option>
        ))}
      </select>
      <ChevronDown
        className="pointer-events-none absolute right-[13px] h-[15px] w-[15px] text-[#969295]"
        strokeWidth={1.6}
      />
    </div>
  );
}
