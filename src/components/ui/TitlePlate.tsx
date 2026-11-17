import { cn } from "@/lib/utils";

/**
 * A small, generated ink plate for search rows. It is decoration, not a
 * fabricated cover: the seal carries the title's initials and the artwork
 * varies deterministically with the source URL.
 */
export function TitlePlate({
  title,
  source,
  className,
}: {
  title: string;
  source: string;
  className?: string;
}) {
  const seed = hash(`${title}·${source}`);
  const variant = seed % 3;
  const seal = initials(title);

  return (
    <div
      className={cn(
        "relative h-[66px] w-[48px] shrink-0 overflow-hidden border border-parchment-100/12 bg-[#12100f]",
        className,
      )}
      aria-hidden="true"
    >
      <svg viewBox="0 0 48 66" className="absolute inset-0 h-full w-full">
        <defs>
          <linearGradient id={`plate-${seed}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#241d18" />
            <stop offset="100%" stopColor="#0b0a0a" />
          </linearGradient>
        </defs>
        <rect width="48" height="66" fill={`url(#plate-${seed})`} />
        <path
          d="M0 18H48M0 33H48M0 48H48"
          stroke="#e8e3d8"
          strokeOpacity="0.03"
        />

        {variant === 0 && (
          <>
            <circle cx="35" cy="14" r="6.5" fill="#963327" opacity="0.55" />
            <path
              d="M0 45 L10 31 L19 41 L29 23 L39 37 L48 29 L48 66 L0 66 Z"
              fill="#08090a"
              opacity="0.92"
            />
            <path
              d="M0 52 C10 47 19 54 28 49 C36 45 43 49 48 46 L48 66 L0 66 Z"
              fill="#050506"
            />
            <path
              d="M0 52 C10 47 19 54 28 49 C36 45 43 49 48 46"
              fill="none"
              stroke="#b5ada0"
              strokeOpacity="0.18"
            />
          </>
        )}

        {variant === 1 && (
          <>
            <circle cx="12" cy="12" r="5.5" fill="#d3ccbf" opacity="0.14" />
            <path
              d="M35 4 C32 18 20 22 15 34 M26 16 C22 29 14 33 11 45 M18 28 C18 38 12 44 9 52"
              fill="none"
              stroke="#8a7d6c"
              strokeOpacity="0.45"
              strokeWidth="1"
            />
            <g fill="#725f4d" opacity="0.5">
              <ellipse cx="29" cy="20" rx="6" ry="1.6" transform="rotate(-26 29 20)" />
              <ellipse cx="18" cy="35" rx="6.5" ry="1.6" transform="rotate(18 18 35)" />
              <ellipse cx="26" cy="41" rx="5" ry="1.4" transform="rotate(-20 26 41)" />
              <ellipse cx="10" cy="22" rx="5" ry="1.4" transform="rotate(24 10 22)" />
            </g>
            <path
              d="M0 55 C12 50 26 58 48 51 L48 66 L0 66 Z"
              fill="#070707"
              opacity="0.9"
            />
          </>
        )}

        {variant === 2 && (
          <>
            <path
              d="M24 10 L36 20 Q34 22 31 22 L17 22 Q14 22 12 20 Z"
              fill="#0a0a0a"
              stroke="#8a7560"
              strokeOpacity="0.35"
            />
            <path
              d="M18 24 L30 24 L31 31 L17 31 Z M20 33 L28 33 L29 39 L19 39 Z"
              fill="#0a0a0a"
              stroke="#8a7560"
              strokeOpacity="0.28"
            />
            <path
              d="M24 5 V10 M21 8 H27"
              stroke="#b5ada0"
              strokeOpacity="0.35"
            />
            <circle cx="10" cy="28" r="7" fill="#77251d" opacity="0.55" />
            <circle cx="40" cy="38" r="5" fill="#963327" opacity="0.4" />
            <path
              d="M0 47 L9 41 L17 46 L27 39 L39 47 L48 43 L48 66 L0 66 Z"
              fill="#070708"
            />
            <path
              d="M3 51 L9 41 M40 54 C43 47 46 44 48 43"
              stroke="#b5ada0"
              strokeOpacity="0.15"
            />
          </>
        )}

        <path
          d="M0 0H48V66H0Z"
          fill="none"
          stroke="#e8e3d8"
          strokeOpacity="0.05"
        />
      </svg>
      <span className="seal absolute bottom-1 right-1 h-[15px] min-w-[15px] px-[2px] text-[9px] tracking-[0.02em]">
        {seal}
      </span>
    </div>
  );
}

function hash(value: string): number {
  let result = 0;
  for (let index = 0; index < value.length; index += 1) {
    result = (result * 31 + value.charCodeAt(index)) >>> 0;
  }
  return result;
}

function initials(title: string): string {
  const words = title
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .split(/\s+/)
    .filter(Boolean);
  if (words.length === 0) return "NF";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return `${words[0][0]}${words[1][0]}`.toUpperCase();
}
