import { cn } from "@/lib/utils";

export function PageHeader({
  eyebrow,
  title,
  description,
  actions,
  className,
}: {
  eyebrow: string;
  title: string;
  description: string;
  actions?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-wrap items-end justify-between gap-x-6 gap-y-3 border-b border-parchment-100/8 pb-4",
        className,
      )}
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2 text-[9.5px] font-semibold uppercase tracking-[0.26em] text-vermilion-300/85">
          <span className="h-[11px] w-[2px] bg-vermilion-500" />
          {eyebrow}
        </div>
        <h1 className="mt-2 font-display text-[38px] font-medium leading-[0.95] tracking-[0.002em] text-parchment-50">
          {title}
        </h1>
        <p className="mt-2 max-w-[580px] text-[13px] leading-relaxed text-parchment-400">
          {description}
        </p>
      </div>
      {actions && (
        <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>
      )}
    </div>
  );
}
