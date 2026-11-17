import { cn } from "@/lib/utils";

/**
 * The NovelForge mark: an open book. Drawn from the line-work used in the
 * desktop reference so the sidebar and About page share one silhouette.
 */
export function ForgeMark({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 40 40"
      fill="none"
      aria-hidden="true"
      className={cn("text-parchment-100", className)}
    >
      <path
        d="M2 6Q12 9 20 18Q28 9 38 6V33Q28 29 20 38Q12 29 2 33Z"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M20 18V38"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}
