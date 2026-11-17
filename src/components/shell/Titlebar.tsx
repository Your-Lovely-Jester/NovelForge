import { Minus, Square, X } from "lucide-react";
import type { Page } from "@/types";
import { isTauriRuntime } from "@/lib/tauri";

async function windowAction(action: "minimize" | "maximize" | "close") {
  if (!isTauriRuntime()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const current = getCurrentWindow();
  if (action === "minimize") await current.minimize();
  if (action === "maximize") await current.toggleMaximize();
  if (action === "close") await current.close();
}

export function Titlebar({ page }: { page: Page; online?: boolean; activeCount?: number }) {
  return (
    <header
      data-tauri-drag-region
      className="relative z-20 flex h-[49px] shrink-0 items-center justify-between border-b border-[#29272a] bg-[#09090b]"
    >
      <div
        data-tauri-drag-region
        className="flex h-full min-w-0 flex-1 items-center pl-4 text-[14px] text-[#777276] sm:pl-6 xl:pl-8"
      >
        <span data-tauri-drag-region className="truncate">
          {PAGE_LABEL[page]}
        </span>
      </div>

      <div
        data-tauri-drag-region
        className="flex h-full items-center self-stretch"
      >
        <TitlebarButton label="Minimize" onClick={() => void windowAction("minimize")}>
          <Minus className="h-[15px] w-[15px]" strokeWidth={1.4} />
        </TitlebarButton>
        <TitlebarButton label="Maximize" onClick={() => void windowAction("maximize")}>
          <Square className="h-[12px] w-[12px]" strokeWidth={1.4} />
        </TitlebarButton>
        <TitlebarButton label="Close" danger onClick={() => void windowAction("close")}>
          <X className="h-[15px] w-[15px]" strokeWidth={1.4} />
        </TitlebarButton>
      </div>
    </header>
  );
}

const PAGE_LABEL: Record<Page, string> = {
  home: "Home",
  webnovels: "Webnovels",
  manga: "Manga / Manhwa",
  books: "EPUB Books",
  library: "Library",
  downloads: "Downloads",
  settings: "Settings",
  guardrails: "Guardrails",
  about: "About",
};

function TitlebarButton({
  children,
  label,
  onClick,
  danger = false,
}: {
  children: React.ReactNode;
  label: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onClick}
      className={`flex h-full w-11 items-center justify-center text-[#aaa6a8] transition-colors ${
        danger
          ? "hover:bg-[#8e353b] hover:text-[#f2e9e9]"
          : "hover:bg-[#1a1a1c] hover:text-[#eee9e8]"
      }`}
    >
      {children}
    </button>
  );
}
