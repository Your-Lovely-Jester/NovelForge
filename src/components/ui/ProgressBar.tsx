import { cn } from "@/lib/utils";

export function ProgressBar({
  value,
  bar,
  active = false,
  size = "md",
  className,
}: {
  value: number;
  bar?: string;
  active?: boolean;
  size?: "sm" | "md";
  className?: string;
}) {
  const clamped = Math.max(0, Math.min(100, Number.isFinite(value) ? value : 0));

  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(clamped)}
      className={cn(
        "relative w-full overflow-hidden bg-parchment-100/[0.07]",
        size === "sm" ? "h-[3px]" : "h-[4px]",
        className,
      )}
    >
      <div
        className={cn(
          "relative h-full overflow-hidden bg-vermilion-500 transition-[width] duration-500 ease-out",
          bar,
          active && "progress-active",
        )}
        style={{ width: `${clamped}%` }}
      />
    </div>
  );
}
