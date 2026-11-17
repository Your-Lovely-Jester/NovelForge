import {
  BookMarked,
  BookOpenText,
  Download,
  Home,
  Images,
  Info,
  Library,
  Settings,
  ShieldCheck,
} from "lucide-react";
import type { Page } from "@/types";
import { cn } from "@/lib/utils";
import { ForgeMark } from "@/components/ui/ForgeMark";

const MAIN: Array<{
  page: Page;
  label: string;
  icon: typeof Home;
  badge?: "active" | "library";
}> = [
  { page: "home", label: "Home", icon: Home },
  { page: "webnovels", label: "Webnovels", icon: BookOpenText },
  { page: "manga", label: "Manga", icon: Images },
  { page: "books", label: "Books", icon: BookMarked },
  { page: "library", label: "Library", icon: Library, badge: "library" },
  { page: "downloads", label: "Downloads", icon: Download, badge: "active" },
];

const STUDIO: Array<{ page: Page; label: string; icon: typeof Home }> = [
  { page: "settings", label: "Settings", icon: Settings },
  { page: "guardrails", label: "Guardrails", icon: ShieldCheck },
  { page: "about", label: "About", icon: Info },
];

export function Sidebar({
  page,
  onNavigate,
  activeCount,
  libraryCount,
}: {
  page: Page;
  onNavigate: (page: Page) => void;
  activeCount: number;
  libraryCount: number;
  libraryBytes: number;
  onOpenDownloads: () => void;
}) {
  return (
    <aside className="relative z-20 flex w-[210px] shrink-0 flex-col overflow-hidden border-r border-[#393639] bg-gradient-to-b from-[#111113] to-[#09090b] max-lg:w-[74px]">
      <div className="flex shrink-0 items-center gap-3 px-[30px] pb-0 pt-[50px] max-lg:justify-center max-lg:px-0">
        <ForgeMark className="h-[34px] w-[34px] shrink-0 text-[#d9d5d6]" />
        <div className="min-w-0 max-lg:hidden">
          <div className="font-display text-[22px] leading-none text-[#eee9e8]">
            NovelForge
          </div>
          <div className="mt-[7px] text-[13px] leading-none text-[#777276]">
            Webnovel Scraper
          </div>
        </div>
      </div>

      <nav className="mt-[52px] flex-1 overflow-y-auto px-[11px] pb-[22px] max-lg:px-2">
        {MAIN.map((item) => (
          <NavItem
            key={item.page}
            item={item}
            active={page === item.page}
            badge={
              item.badge === "active"
                ? activeCount
                : item.badge === "library"
                  ? libraryCount
                  : 0
            }
            onNavigate={onNavigate}
          />
        ))}

        <div className="my-[18px] h-px bg-[#29272a]" />

        {STUDIO.map((item) => (
          <NavItem
            key={item.page}
            item={item}
            active={page === item.page}
            badge={0}
            onNavigate={onNavigate}
          />
        ))}
      </nav>
    </aside>
  );
}

function NavItem({
  item,
  active,
  badge,
  onNavigate,
}: {
  item: { page: Page; label: string; icon: typeof Home; badge?: "active" | "library" };
  active: boolean;
  badge: number;
  onNavigate: (page: Page) => void;
}) {
  const Icon = item.icon;
  return (
    <button
      type="button"
      onClick={() => onNavigate(item.page)}
      className={cn(
        "group relative mb-2 flex h-[46px] w-full items-center gap-[14px] rounded-[2px] pl-[18px] pr-3 text-left text-[16px] transition-colors last:mb-0 max-lg:justify-center max-lg:gap-0 max-lg:pl-0 max-lg:pr-0",
        active
          ? "bg-[#251a1b] text-[#e8e3e4]"
          : "text-[#d8d6d7] hover:bg-[#151416] hover:text-[#eee9e8]",
      )}
      aria-label={item.label}
      title={item.label}
    >
      {active && (
        <span className="absolute left-0 top-0 h-full w-[5px] bg-[#a94349]" />
      )}
      <Icon
        className={cn(
          "h-[18px] w-[18px] shrink-0",
          active ? "text-[#d9d5d6]" : "text-[#c8c5c6] group-hover:text-[#e8e3e4]",
        )}
        strokeWidth={1.6}
      />
      <span className="flex-1 truncate max-lg:hidden">{item.label}</span>
      {badge > 0 && (
        <span
          className={cn(
            "min-w-[20px] rounded-[3px] px-1.5 py-px text-center font-mono text-[13px] tabular-nums max-lg:absolute max-lg:right-0.5 max-lg:top-0.5 max-lg:min-w-0 max-lg:px-1 max-lg:text-[10px]",
            item.badge === "active"
              ? "bg-[#3a1e21] text-[#d69a9e]"
              : "text-[#777276]",
          )}
        >
          {badge}
        </span>
      )}
    </button>
  );
}
