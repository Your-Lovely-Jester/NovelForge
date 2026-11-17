import { FolderOpen } from "lucide-react";
import type { LibraryNovel } from "@/types";
import { formatBytes } from "@/lib/utils";

export function LibraryCard({
  novels,
  totalBytes,
  onOpen,
  onOpenFolder,
}: {
  novels: LibraryNovel[];
  totalBytes: number;
  onOpen: () => void;
  onOpenFolder: () => void;
}) {
  const chapters = novels.reduce((sum, novel) => sum + novel.downloadedChapters, 0);

  return (
    <section className="rounded-[5px] border border-[#302d30] bg-gradient-to-b from-[#111113] to-[#09090b] px-5 pb-5 pt-[22px]">
      <div className="flex items-center gap-2.5">
        <ShelfGlyph />
        <h2 className="font-display text-[22px] leading-none text-[#eee9e8]">Library</h2>
        <button
          type="button"
          onClick={onOpenFolder}
          aria-label="Open download folder"
          title="Open download folder"
          className="ml-auto flex h-6 w-6 items-center justify-center text-[#777276] transition-colors hover:text-[#d8d6d7]"
        >
          <FolderOpen className="h-[15px] w-[15px]" strokeWidth={1.6} />
        </button>
      </div>

      <dl className="mt-5 space-y-[13px]">
        <LibraryRow label="Total Novels" value={String(novels.length)} />
        <LibraryRow
          label="Downloaded Chapters"
          value={chapters.toLocaleString()}
        />
        <LibraryRow label="Storage Used" value={formatBytes(totalBytes)} />
      </dl>

      <button
        type="button"
        onClick={onOpen}
        className="mt-5 flex h-[42px] w-full items-center justify-center rounded-[4px] border border-[#3a373a] bg-[#101012] text-[16px] text-[#d8d6d7] transition-colors hover:border-[#514d4f] hover:bg-[#151416] hover:text-[#eee9e8]"
      >
        Open Library
      </button>
    </section>
  );
}

function LibraryRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="shrink-0 text-[15px] text-[#969295]">{label}</dt>
      <dd className="truncate text-[15px] tabular-nums text-[#d8d6d7]" title={value}>
        {value}
      </dd>
    </div>
  );
}

function ShelfGlyph() {
  return (
    <svg viewBox="0 0 20 20" className="h-[18px] w-[18px] text-[#d8d6d7]" fill="none" aria-hidden="true">
      <path d="M4 4h5v12H4zM11 4h5v12h-5z" stroke="currentColor" strokeWidth="1.4" />
    </svg>
  );
}
