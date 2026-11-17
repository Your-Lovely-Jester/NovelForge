import { cn } from "@/lib/utils";
import { chapterHint, parseChapterSpec } from "@/lib/chapters";

const PRESETS = [
  { label: "All", value: "all" },
  { label: "1–20", value: "1-20" },
  { label: "19,31,82", value: "19,31,82" },
];

export function CrawlOptions({
  chapters,
  onChaptersChange,
  jobs,
  onJobsChange,
  throttle,
  onThrottleChange,
  maxJobs = 8,
  throttleMode = "batch-delay",
  className,
}: {
  chapters: string;
  onChaptersChange: (value: string) => void;
  jobs?: number;
  onJobsChange?: (value: number) => void;
  throttle: boolean;
  onThrottleChange: (value: boolean) => void;
  maxJobs?: number;
  throttleMode?: "batch-delay" | "rate-limit";
  className?: string;
}) {
  const selection = parseChapterSpec(chapters);
  const showJobs = jobs !== undefined && onJobsChange !== undefined;

  return (
    <div className={cn("crawl-options-host", className)}>
      <div className={cn("crawl-options", !showJobs && "crawl-options--no-jobs")}>
        <div className="crawl-options__chapters">
          <div className="mb-1.5 flex items-center justify-between gap-2">
            <span className="text-[9.5px] font-semibold uppercase tracking-[0.16em] text-parchment-500">
              Chapter specification
            </span>
            <span className="hidden text-[9.5px] text-parchment-600 sm:block">
              single · range · list
            </span>
          </div>
          <div
            className={cn(
              "field h-9 gap-2 px-2.5",
              !selection.valid && "border-vermilion-500/60",
            )}
          >
            <input
              value={chapters}
              onChange={(event) => onChaptersChange(event.target.value)}
              spellCheck={false}
              autoComplete="off"
              placeholder="all · 1-20 · 19,31,82"
              aria-label="Chapter specification"
              className="h-full font-mono text-[12.5px]"
            />
          </div>
          <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
            {PRESETS.map((preset) => (
              <button
                key={preset.value}
                type="button"
                onClick={() => onChaptersChange(preset.value)}
                className={cn(
                  "h-[17px] border px-1.5 text-[9.5px] font-medium uppercase tracking-[0.08em] transition-colors",
                  chapters === preset.value
                    ? "border-vermilion-500/45 bg-vermilion-500/10 text-vermilion-200"
                    : "border-parchment-100/9 text-parchment-500 hover:border-parchment-100/20 hover:text-parchment-200",
                )}
              >
                {preset.label}
              </button>
            ))}
            <span
              className={cn(
                "ml-1 truncate text-[10px]",
                selection.valid ? "text-parchment-600" : "text-vermilion-300",
              )}
            >
              {chapterHint(selection)}
              {selection.valid && !selection.all && selection.normalized
                ? ` → ${selection.normalized}`
                : ""}
            </span>
          </div>
        </div>

        {showJobs && (
          <div className="crawl-options__jobs">
            <div className="mb-1.5 flex items-center justify-between">
              <span className="text-[9.5px] font-semibold uppercase tracking-[0.16em] text-parchment-500">
                Parallel jobs
              </span>
              <span className="font-mono text-[11px] tabular-nums text-parchment-200">
                {jobs}
              </span>
            </div>
            <div className="flex h-9 items-center">
              <input
                type="range"
                min={1}
                max={maxJobs}
                step={1}
                value={jobs}
                onChange={(event) => onJobsChange(Number(event.target.value))}
                aria-label="Parallel jobs"
                className="input-range"
              />
            </div>
            <div className="mt-1.5 text-[10px] text-parchment-600">
              Concurrent downloads.
            </div>
          </div>
        )}

        <div className="crawl-options__politeness">
          <span className="text-[9.5px] font-semibold uppercase tracking-[0.16em] text-parchment-500">
            Politeness
          </span>
          <button
            type="button"
            onClick={() => onThrottleChange(!throttle)}
            aria-pressed={throttle}
            className="mt-1.5 flex h-9 w-full items-center justify-between border border-parchment-100/9 px-2 text-left transition-colors hover:border-parchment-100/18"
          >
            <span className="text-[11px] text-parchment-300">Throttle</span>
            <span className="toggle" data-on={throttle} />
          </button>
          <div className="mt-1.5 text-[10px] text-parchment-600">
            {throttleMode === "rate-limit"
              ? throttle
                ? "Caps source requests at 2/sec."
                : "Uses the source's default pacing."
              : throttle
                ? "Delay between batches."
                : "Maximum throughput."}
          </div>
        </div>
      </div>
    </div>
  );
}
