import {
  Download,
  FolderOpen,
  Gauge,
  Loader2,
  Pause,
  Play,
  TerminalSquare,
  X,
} from "lucide-react";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import {
  cn,
  formatBytes,
  formatDuration,
  formatPace,
  formatSpeed,
} from "@/lib/utils";
import { PageHeader } from "@/components/ui/PageHeader";
import { ProgressBar } from "@/components/ui/ProgressBar";
import { TerminalDock } from "@/components/shell/TerminalDock";

const STATUS_TONE: Record<string, string> = {
  running: "text-vermilion-300",
  verifying: "text-gold-300",
  paused: "text-parchment-300",
  queued: "text-parchment-400",
  completed: "text-jade-300",
  failed: "text-vermilion-300",
  cancelled: "text-parchment-500",
  interrupted: "text-amber-200",
};

export function DownloadsPage({ manager }: { manager: DownloadManager }) {
  const active = manager.activeTasks;
  const sessionBytes = manager.tasks.reduce((total, task) => total + task.bytes, 0);

  return (
    <div className="space-y-4">
      <PageHeader
        eyebrow="Studio output"
        title="Downloads"
        description="Live transfers, the waiting queue, and the full engine log. Each process writes into the local download sandbox before verification."
        actions={
          <button
            type="button"
            onClick={() => void manager.openDownloads()}
            className="btn-secondary"
          >
            <FolderOpen className="h-3.5 w-3.5" strokeWidth={1.8} />
            Open folder
          </button>
        }
      />

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <Metric label="Active" value={String(active.length)} accent />
        <Metric label="Session data" value={formatBytes(sessionBytes)} />
        <Metric label="Log lines" value={String(manager.logs.length)} />
      </div>

      <section className="panel">
        <div className="flex items-center gap-2 border-b border-parchment-100/8 px-5 py-3">
          <Download className="h-4 w-4 text-vermilion-400" strokeWidth={1.8} />
          <h2 className="font-display text-[20px] text-parchment-50">Transfers</h2>
          <span className="ml-auto text-[15px] text-parchment-500">
            {manager.tasks.length} total
          </span>
        </div>

        {manager.tasks.length === 0 ? (
          <div className="px-5 py-10 text-center text-[15px] text-parchment-500">
            No transfers yet. Start a novel, manga, or book download.
          </div>
        ) : (
          <div className="divide-y divide-[#242124]">
            {manager.tasks.map((task) => {
              const running = task.status === "running";
              const paused = task.status === "paused";
              const interrupted = task.status === "interrupted";
              return (
                <div key={task.id} className="flex items-center gap-4 px-5 py-3.5">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-baseline gap-2.5">
                      <span className="truncate text-[16px] text-parchment-100">
                        {task.title}
                      </span>
                      <span className="shrink-0 text-[14px] capitalize text-parchment-500">
                        {task.engine === "webnovel" ? "novel" : task.engine}
                      </span>
                    </div>
                    <div className="mt-1.5 flex items-center gap-3">
                      <ProgressBar
                        value={task.progress}
                        active={running || task.status === "verifying"}
                        className="max-w-[380px]"
                      />
                      <span className="w-10 shrink-0 text-right font-mono text-[14px] tabular-nums text-parchment-400">
                        {Math.round(task.progress)}%
                      </span>
                      <span className="hidden shrink-0 text-[14px] text-parchment-500 sm:block">
                        {formatBytes(task.bytes)}
                        {task.speedBps > 0 && ` · ${formatSpeed(task.speedBps)}`}
                        {task.etaSeconds != null && ` · ${formatDuration(task.etaSeconds)} left`}
                        {running &&
                          task.idleSeconds >= 10 &&
                          ` · no data for ${formatDuration(task.idleSeconds)}`}
                      </span>
                    </div>
                    {task.expectedChaptersPerMinute != null && (
                      <div className="mt-1 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[12px] text-parchment-600">
                        <Gauge
                          className="h-3 w-3 shrink-0 text-jade-400/70"
                          strokeWidth={1.9}
                        />
                        <span>
                          Measured{" "}
                          <span className="font-mono tabular-nums text-parchment-400">
                            {formatPace(task.expectedChaptersPerMinute)}
                          </span>{" "}
                          on this source
                        </span>
                        {task.expectedSeconds != null && task.expectedSeconds > 0 && (
                          <span>
                            · expected crawl{" "}
                            {formatDuration(task.expectedSeconds)}
                          </span>
                        )}
                      </div>
                    )}
                  </div>

                  <div className="flex shrink-0 items-center gap-1.5">
                    <span className={cn("mr-1.5 hidden text-[14px] md:block", STATUS_TONE[task.status])}>
                      {task.message ?? task.status}
                    </span>
                    {interrupted && (
                      <button
                        type="button"
                        onClick={() => void manager.resumeInterrupted(task)}
                        className="icon-button h-8 w-8 text-amber-200"
                        aria-label="Resume interrupted download"
                        title="Resume interrupted download"
                      >
                        <Play className="h-3.5 w-3.5" strokeWidth={1.8} />
                      </button>
                    )}
                    {(running || paused) && (
                      <button
                        type="button"
                        onClick={() => void (running ? manager.pause(task.id) : manager.resume(task.id))}
                        className="icon-button h-8 w-8"
                        aria-label={running ? "Pause" : "Resume"}
                        title={running ? "Pause" : "Resume"}
                      >
                        {running ? (
                          <Pause className="h-3.5 w-3.5" strokeWidth={1.8} />
                        ) : (
                          <Play className="h-3.5 w-3.5" strokeWidth={1.8} />
                        )}
                      </button>
                    )}
                    <button
                      type="button"
                      onClick={() => manager.setActiveJobId(task.id)}
                      className="icon-button h-8 w-8"
                      aria-label="Inspect log"
                      title="Inspect log"
                    >
                      {task.status === "verifying" ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" strokeWidth={1.8} />
                      ) : (
                        <TerminalSquare className="h-3.5 w-3.5" strokeWidth={1.8} />
                      )}
                    </button>
                    <button
                      type="button"
                      onClick={() => void manager.cancel(task.id)}
                      className="icon-button h-8 w-8 hover:text-vermilion-300"
                      aria-label="Cancel"
                      title="Cancel"
                    >
                      <X className="h-3.5 w-3.5" strokeWidth={1.8} />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>

      <section className="panel overflow-hidden">
        <div className="flex items-center gap-2 border-b border-parchment-100/8 px-5 py-3">
          <TerminalSquare className="h-4 w-4 text-vermilion-400" strokeWidth={1.8} />
          <h2 className="font-display text-[20px] text-parchment-50">Engine log</h2>
          <span className="ml-auto text-[15px] text-parchment-500">
            stdout · stderr · verification
          </span>
        </div>
        <TerminalDock
          logs={manager.logs}
          tasks={manager.tasks}
          activeJobId={manager.activeJobId}
          onSelectJob={manager.setActiveJobId}
          onClear={manager.clearLogs}
          onClearFinished={() => void manager.clearFinished()}
          className="h-[380px] border-t-0"
        />
      </section>
    </div>
  );
}

function Metric({
  label,
  value,
  accent = false,
}: {
  label: string;
  value: string;
  accent?: boolean;
}) {
  return (
    <div className="panel px-5 py-3.5">
      <div className="text-[15px] text-parchment-500">{label}</div>
      <div
        className={cn(
          "mt-1 font-display text-[25px] leading-none",
          accent ? "text-vermilion-300" : "text-parchment-100",
        )}
      >
        {value}
      </div>
    </div>
  );
}
