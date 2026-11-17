import { Pause, Play } from "lucide-react";
import type { DownloadTask } from "@/types";
import { formatDuration, formatSpeed, phaseLabel } from "@/lib/utils";

export function ProgressCard({
  percent,
  activeTasks,
  sessionBytes,
  currentTask,
  onInspect,
  onPause,
  onResume,
}: {
  percent: number;
  activeTasks: DownloadTask[];
  sessionBytes: number;
  currentTask: DownloadTask | null;
  onInspect: () => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
}) {
  const running = currentTask?.status === "running";
  const paused = currentTask?.status === "paused";
  const elapsed = currentTask?.elapsedSeconds ?? 0;
  const phase = phaseLabel(currentTask?.phase);
  const idle = currentTask?.idleSeconds ?? 0;
  const remaining = currentTask?.etaSeconds ?? null;
  const speed = activeTasks
    .filter((task) => task.status === "running")
    .reduce((total, task) => total + task.speedBps, 0);
  const value = Math.max(0, Math.min(100, percent));

  return (
    <section className="rounded-[5px] border border-[#302d30] bg-gradient-to-b from-[#111113] to-[#09090b] px-5 pb-5 pt-[22px]">
      <RailHeading icon={<ProgressGlyph />} title="Scraping Progress" />

      <div className="mt-5 flex items-baseline justify-between gap-3">
        <span className="text-[15px] text-[#969295]">Overall Progress</span>
        <span className="text-[15px] tabular-nums text-[#d8d6d7]">
          {Math.round(value)}%
        </span>
      </div>
      <div className="mt-2.5 h-[11px] overflow-hidden rounded-[2px] bg-[#292629]">
        <div
          className="relative h-full overflow-hidden rounded-[2px] bg-[#8e353b] transition-[width] duration-500 ease-out"
          style={{ width: `${value}%` }}
        >
          {(running || currentTask?.status === "verifying") && (
            <span className="progress-active absolute inset-0" />
          )}
        </div>
      </div>

      <dl className="mt-[22px] space-y-[13px]">
        <RailRow
          label="Current Task"
          value={currentTask ? currentTask.title : "No active transfer"}
          onClick={currentTask ? onInspect : undefined}
        />
        <RailRow
          label="Phase"
          value={
            running && idle >= 10 ? `${phase} · ${idle}s idle` : phase
          }
        />
        <RailRow label="Elapsed Time" value={formatDuration(elapsed)} />
        <RailRow
          label="Estimated Time"
          value={remaining != null ? formatDuration(remaining) : "—"}
        />
        <RailRow label="Speed" value={speed > 0 ? formatSpeed(speed) : "—"} />
      </dl>

      <button
        type="button"
        disabled={!running && !paused}
        onClick={() => {
          if (!currentTask) return;
          if (running) onPause(currentTask.id);
          else if (paused) onResume(currentTask.id);
        }}
        className="mt-6 flex h-[42px] w-full items-center justify-center gap-2 rounded-[4px] border border-[#8d3b40] bg-[#151113] text-[16px] text-[#d8d6d7] transition-colors hover:border-[#a94349] hover:bg-[#1d1416] disabled:border-[#302d30] disabled:bg-transparent disabled:text-[#5d595c]"
      >
        {paused ? (
          <Play className="h-[15px] w-[15px]" strokeWidth={1.7} />
        ) : (
          <Pause className="h-[15px] w-[15px]" strokeWidth={1.7} />
        )}
        {paused ? "Resume" : running ? "Pause" : "Idle"}
      </button>

      <div className="mt-3 flex items-center justify-between text-[14px] text-[#777276]">
        <span>
          {activeTasks.length > 0
            ? `${activeTasks.length} active ${activeTasks.length === 1 ? "job" : "jobs"}`
            : "No active jobs"}
        </span>
        <span className="tabular-nums">{formatSize(sessionBytes)} session</span>
      </div>
    </section>
  );
}

function RailHeading({ icon, title }: { icon: React.ReactNode; title: string }) {
  return (
    <div className="flex items-center gap-2.5">
      <span className="text-[#d8d6d7]">{icon}</span>
      <h2 className="font-display text-[22px] leading-none text-[#eee9e8]">{title}</h2>
    </div>
  );
}

function RailRow({
  label,
  value,
  onClick,
}: {
  label: string;
  value: string;
  onClick?: () => void;
}) {
  const content = (
    <>
      <dt className="shrink-0 text-[15px] text-[#969295]">{label}</dt>
      <dd
        className={
          onClick
            ? "min-w-0 truncate text-right text-[15px] text-[#d8d6d7] transition-colors hover:text-[#eee9e8]"
            : "min-w-0 truncate text-right text-[15px] text-[#d8d6d7]"
        }
        title={value}
      >
        {value}
      </dd>
    </>
  );

  if (onClick) {
    return (
      <button
        type="button"
        onClick={onClick}
        className="flex w-full items-baseline justify-between gap-3 text-left"
      >
        {content}
      </button>
    );
  }
  return <div className="flex items-baseline justify-between gap-3">{content}</div>;
}

function ProgressGlyph() {
  return (
    <svg viewBox="0 0 20 20" className="h-[18px] w-[18px]" fill="none" aria-hidden="true">
      <path d="M4 3h12v14H4z" stroke="currentColor" strokeWidth="1.4" />
      <path d="M7 13V9M10 13V6M13 13v-3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

function formatSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** exponent).toFixed(exponent === 0 ? 0 : 1)} ${units[exponent]}`;
}
