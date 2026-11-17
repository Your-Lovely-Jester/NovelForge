import { Clock3, X } from "lucide-react";
import type { DownloadTask } from "@/types";
import { cn, phaseLabel } from "@/lib/utils";

export function QueueCard({
  queue,
  onSelect,
  onCancel,
}: {
  queue: DownloadTask[];
  onSelect: (id: string) => void;
  onCancel: (id: string) => void;
}) {
  return (
    <section className="rounded-[5px] border border-[#302d30] bg-gradient-to-b from-[#111113] to-[#09090b] px-5 pb-5 pt-[22px]">
      <div className="flex items-center gap-2.5">
        <ListGlyph />
        <h2 className="font-display text-[22px] leading-none text-[#eee9e8]">Queue</h2>
        <span className="ml-auto text-[15px] tabular-nums text-[#777276]">
          {queue.length.toString().padStart(2, "0")}
        </span>
      </div>

      {queue.length === 0 ? (
        <div className="py-8 text-center">
          <div className="text-[15px] text-[#969295]">Queue is clear</div>
          <div className="mt-1 text-[14px] text-[#777276]">
            New transfers appear here.
          </div>
        </div>
      ) : (
        <div className="mt-5 space-y-[18px]">
          {queue.map((task) => {
            const running = task.status === "running";
            const verifying = task.status === "verifying";
            return (
              <div key={task.id}>
                <div className="flex items-start gap-2">
                  <span
                    className={cn(
                      "mt-[3px] h-[15px] w-[3px] shrink-0 rounded-full",
                      running
                        ? "bg-[#9b3b41]"
                        : verifying
                          ? "bg-[#c28b65]"
                          : "bg-[#3a373a]",
                    )}
                  />
                  <button
                    type="button"
                    onClick={() => onSelect(task.id)}
                    className="min-w-0 flex-1 text-left"
                  >
                    <div className="truncate text-[15px] text-[#d8d6d7] transition-colors hover:text-[#eee9e8]">
                      {task.title}
                    </div>
                    <div className="mt-0.5 flex items-center gap-1.5 text-[15px] text-[#969295]">
                      {!running && !verifying && (
                        <Clock3 className="h-3 w-3 shrink-0 text-[#777276]" strokeWidth={1.7} />
                      )}
                      <span
                        className={cn(
                          "h-1 w-1 shrink-0 rounded-full",
                          running
                            ? "bg-[#b14a50]"
                            : verifying
                              ? "bg-[#c28b65]"
                              : "bg-[#5d595c]",
                        )}
                      />
                      <span className="truncate">{statusLabel(task)}</span>
                    </div>
                  </button>
                  <button
                    type="button"
                    onClick={() => onCancel(task.id)}
                    aria-label={`Cancel ${task.title}`}
                    title="Cancel transfer"
                    className="mt-px flex h-5 w-5 shrink-0 items-center justify-center text-[#777276] transition-colors hover:text-[#d8d6d7]"
                  >
                    <X className="h-[17px] w-[17px]" strokeWidth={1.6} />
                  </button>
                </div>

                {(running || verifying || task.status === "paused") && (
                  <div className="ml-[11px] mt-2 h-[5px] overflow-hidden rounded-[2px] bg-[#292629]">
                    <div
                      className={cn(
                        "h-full rounded-[2px] transition-[width] duration-500",
                        verifying ? "bg-[#c28b65]" : "bg-[#963b40]",
                      )}
                      style={{ width: `${task.progress}%` }}
                    />
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}

function statusLabel(task: DownloadTask): string {
  switch (task.status) {
    case "running":
      return `${phaseLabel(task.phase)} · ${Math.round(task.progress)}%`;
    case "verifying":
      return `${phaseLabel(task.phase)}…`;
    case "paused":
      return "Paused";
    case "queued":
      return "Queued";
    default:
      return task.message ?? task.status;
  }
}

function ListGlyph() {
  return (
    <svg viewBox="0 0 20 20" className="h-[18px] w-[18px] text-[#d8d6d7]" fill="none" aria-hidden="true">
      <path d="M4 5h12M4 10h12M4 15h8" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}
