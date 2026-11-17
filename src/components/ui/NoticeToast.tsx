import { AlertTriangle, CheckCircle2, Info, X } from "lucide-react";
import type { Notice } from "@/hooks/useDownloadManager";
import { cn } from "@/lib/utils";

const TONES = {
  success: {
    icon: CheckCircle2,
    ring: "border-emerald-400/25",
    iconTone: "text-emerald-300",
    glow: "shadow-[0_24px_60px_-30px_rgba(16,185,129,0.55)]",
    rail: "bg-emerald-400",
  },
  error: {
    icon: AlertTriangle,
    ring: "border-rose-500/30",
    iconTone: "text-rose-300",
    glow: "shadow-[0_24px_60px_-30px_rgba(244,63,94,0.6)]",
    rail: "bg-rose-500",
  },
  info: {
    icon: Info,
    ring: "border-sky-400/25",
    iconTone: "text-sky-300",
    glow: "shadow-[0_24px_60px_-30px_rgba(56,189,248,0.5)]",
    rail: "bg-sky-400",
  },
} as const;

export function NoticeToast({
  notice,
  onClose,
}: {
  notice: Notice;
  onClose: () => void;
}) {
  const tone = TONES[notice.kind];
  const Icon = tone.icon;

  return (
    <div
      role="status"
      aria-live="polite"
      className={cn(
        "pointer-events-auto relative w-[322px] overflow-hidden rounded-xl border bg-[#141418]/95 p-3.5 pl-4 backdrop-blur-2xl animate-fade-up",
        tone.ring,
        tone.glow,
      )}
    >
      <span
        className={cn("absolute inset-y-0 left-0 w-[2px]", tone.rail)}
        aria-hidden="true"
      />
      <div className="flex gap-2.5">
        <Icon
          className={cn("mt-[1px] h-4 w-4 shrink-0", tone.iconTone)}
          strokeWidth={1.9}
        />
        <div className="min-w-0 flex-1">
          <div className="text-[14.5px] font-semibold text-slate-100">
            {notice.title}
          </div>
          {notice.message && (
            <div className="mt-0.5 text-[13.5px] leading-relaxed text-slate-400">
              {notice.message}
            </div>
          )}
        </div>
        <button
          type="button"
          aria-label="Dismiss notification"
          onClick={onClose}
          className="icon-button -mr-1 -mt-1 h-6 w-6"
        >
          <X className="h-3 w-3" strokeWidth={2} />
        </button>
      </div>
    </div>
  );
}
