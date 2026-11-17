import type { DownloadManager } from "@/hooks/useDownloadManager";
import { PageHeader } from "@/components/ui/PageHeader";
import { NovelSearchPanel } from "@/components/novels/NovelSearchPanel";

export function WebnovelPage({
  manager,
  initialQuery,
  searchNonce,
}: {
  manager: DownloadManager;
  initialQuery: string;
  searchNonce: number;
}) {
  return (
    <div className="space-y-4">
      <PageHeader
        eyebrow="Collection 01 / Webnovels"
        title="Webnovels"
        description="Search every enabled source, a chosen site, or paste a direct novel URL. Choose any chapter, a range, or an exact list — there is no chapter cap."
        actions={
          <>
            <span className="chip">novelfire.net</span>
            <span className="chip">novelphoenix.net</span>
            <span className="chip">novelfull.net</span>
            <span className="chip chip-vermilion">EPUB</span>
          </>
        }
      />

      <NovelSearchPanel
        manager={manager}
        initialQuery={initialQuery}
        searchNonce={searchNonce}
      />

      <p className="px-1 text-[10.5px] leading-relaxed text-parchment-600">
        Chapter specifications are normalised before crawling: duplicates are
        removed, ranges are compressed, and chapters are written in ascending
        order. The crawler has no 100-chapter ceiling.
      </p>
    </div>
  );
}
