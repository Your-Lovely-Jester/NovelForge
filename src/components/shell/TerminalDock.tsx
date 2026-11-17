import { useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronDown,
  Circle,
  Eraser,
  PanelBottomClose,
  PanelBottomOpen,
  TerminalSquare,
} from "lucide-react";
import type { DownloadTask, LogEvent } from "@/types";
import { cn, formatClock, isActive } from "@/lib/utils";

export function TerminalDock({
  logs,
  tasks,
  activeJobId,
  onSelectJob,
  onClear,
  className,
}: {
  logs: LogEvent[];
  tasks: DownloadTask[];
  activeJobId: string | null;
  onSelectJob: (id: string | null) => void;
  onClear: () => void;
  className?: string;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const [pinned, setPinned] = useState(true);
  const scrollRef = useRef<HTMLDivElement>(null);

  const visible = useMemo(() => {
    const filtered = activeJobId
      ? logs.filter((entry) => entry.jobId === activeJobId)
      : logs;
    return filtered.slice(-400);
  }, [activeJobId, logs]);

  const active = tasks.some((task) => isActive(task.status));

  useEffect(() => {
    if (!pinned || collapsed) return;
    const node = scrollRef.current;
    if (!node) return;
    node.scrollTop = node.scrollHeight;
  }, [visible, pinned, collapsed]);

  function handleScroll() {
    const node = scrollRef.current;
    if (!node) return;
    const distance = node.scrollHeight - node.scrollTop - node.clientHeight;
    setPinned(distance < 32);
  }

  const selected = tasks.find((task) => task.id === activeJobId);

  return (
    <section
      className={cn(
        "relative z-10 flex shrink-0 flex-col border-t border-parchment-100/9 transition-[height] duration-300",
        collapsed ? "h-[36px]" : className ?? "h-[184px]",
      )}
    >
      <div className="flex h-[36px] shrink-0 items-center justify-between gap-3 border-b border-parchment-100/7 bg-[#08090a] pl-4 pr-2">
        <div className="flex min-w-0 items-center gap-2.5">
          <TerminalSquare className="h-3.5 w-3.5 text-vermilion-400" strokeWidth={1.8} />
          <span className="text-[10.5px] font-semibold uppercase tracking-[0.2em] text-parchment-300">
            Terminal
          </span>
          <span
            className={cn(
              "flex items-center gap-1.5 border px-1.5 py-[1px] text-[9.5px] font-semibold uppercase tracking-[0.12em]",
              active
                ? "border-vermilion-500/30 bg-vermilion-500/8 text-vermilion-200"
                : "border-parchment-100/9 text-parchment-600",
            )}
          >
            <span
              className={cn(
                "h-1 w-1",
                active ? "animate-pulse-soft bg-vermilion-400" : "bg-parchment-600",
              )}
            />
            {active ? "streaming" : "idle"}
          </span>
          <span className="hidden font-mono text-[10.5px] tabular-nums text-parchment-600 sm:block">
            {visible.length.toString().padStart(4, "0")} lines
          </span>
        </div>

        <div className="flex items-center gap-1.5">
          <div className="relative">
            <select
              value={activeJobId ?? "all"}
              onChange={(event) =>
                onSelectJob(event.target.value === "all" ? null : event.target.value)
              }
              aria-label="Filter terminal output by job"
              className="h-6 max-w-[190px] appearance-none truncate border border-parchment-100/10 bg-transparent pl-2 pr-6 text-[11px] text-parchment-400 outline-none transition-colors hover:border-parchment-100/22 hover:text-parchment-100"
            >
              <option value="all" className="bg-[#121214]">
                All jobs · {tasks.length}
              </option>
              {tasks.map((task) => (
                <option key={task.id} value={task.id} className="bg-[#121214]">
                  {task.title}
                </option>
              ))}
            </select>
            <ChevronDown className="pointer-events-none absolute right-2 top-1/2 h-2.5 w-2.5 -translate-y-1/2 text-parchment-600" />
          </div>

          <button
            type="button"
            onClick={onClear}
            disabled={visible.length === 0}
            className="icon-button h-6 w-6"
            aria-label="Clear terminal"
            title="Clear terminal"
          >
            <Eraser className="h-3 w-3" strokeWidth={1.8} />
          </button>
          <button
            type="button"
            onClick={() => setCollapsed((current) => !current)}
            className="icon-button h-6 w-6"
            aria-label={collapsed ? "Expand terminal" : "Collapse terminal"}
            title={collapsed ? "Expand terminal" : "Collapse terminal"}
          >
            {collapsed ? (
              <PanelBottomOpen className="h-3.5 w-3.5" strokeWidth={1.8} />
            ) : (
              <PanelBottomClose className="h-3.5 w-3.5" strokeWidth={1.8} />
            )}
          </button>
        </div>
      </div>

      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className={cn(
          "terminal-surface min-h-0 flex-1 overflow-y-auto px-3 py-2 font-mono text-[12px] leading-[1.8]",
          collapsed && "hidden",
        )}
      >
        {visible.length === 0 ? (
          <div className="flex h-full flex-col justify-end gap-1 pb-1">
            <div className="text-parchment-600">
              <span className="text-vermilion-400">novelforge</span>
              <span className="text-parchment-700"> ~ </span>
              <span className="text-parchment-500">
                {selected
                  ? `no output yet for ${selected.title}`
                  : "ready — engine output streams here"}
              </span>
              <span className="terminal-cursor" />
            </div>
            <div className="flex items-center gap-1.5 text-[10.5px] text-parchment-700">
              <Circle className="h-1.5 w-1.5 fill-current" />
              stdout · stderr · verification events
            </div>
          </div>
        ) : (
          visible.map((entry, index) => (
            <div
              key={`${entry.ts}-${index}`}
              className="terminal-line -mx-1 flex gap-2 px-1 py-[1px]"
            >
              <span className="shrink-0 tabular-nums text-parchment-700">
                {formatClock(entry.ts)}
              </span>
              <JobTag task={tasks.find((task) => task.id === entry.jobId)} />
              <span
                className={cn(
                  "min-w-0 flex-1 whitespace-pre-wrap break-words",
                  lineTone(entry),
                )}
              >
                {entry.stream === "system" && (
                  <span className="mr-1 text-vermilion-400/70">›</span>
                )}
                {entry.line}
              </span>
            </div>
          ))
        )}
      </div>

      {!collapsed && (
        <div className="pointer-events-none absolute inset-x-0 bottom-0 h-8 bg-gradient-to-t from-[#070708] to-transparent" />
      )}
    </section>
  );
}

function JobTag({ task }: { task?: DownloadTask }) {
  if (!task) {
    return <span className="w-[44px] shrink-0 text-parchment-700">system</span>;
  }
  return (
    <span className="w-[44px] shrink-0 truncate text-[10px] uppercase tracking-[0.05em] text-parchment-600">
      {task.engine === "webnovel" ? "novel" : task.engine}
    </span>
  );
}

function lineTone(entry: LogEvent): string {
  if (entry.stream === "stderr") return "text-vermilion-300/90";
  if (entry.line.startsWith("SECURITY") || entry.line.startsWith("Failure")) {
    return "text-vermilion-300/90";
  }
  if (entry.line.startsWith("Success") || entry.line.startsWith("Verified")) {
    return "text-jade-300/85";
  }
  if (entry.stream === "system") return "text-parchment-300/90";
  return "text-parchment-400/80";
}
