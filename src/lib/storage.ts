import type { LibraryNovel } from "@/types";

/** Fallback when the library has no completed download to measure. Roughly
 * 18 KB per chapter file plus its share of the bundled EPUB. */
export const FALLBACK_BYTES_PER_CHAPTER = 18 * 1024;

/** Warn before an all-chapters run at or above this size. */
export const LARGE_DOWNLOAD_CHAPTERS = 300;

export interface SizeEstimate {
  bytes: number;
  bytesPerChapter: number;
  /** `library` means the average came from real downloaded folders. */
  basis: "library" | "fallback";
}

/** Average bytes per downloaded chapter across the shelf, clamped to a
 * believable range so one image-heavy manga folder cannot distort a novel
 * estimate. */
export function averageChapterBytes(novels: LibraryNovel[]): number | null {
  let bytes = 0;
  let chapters = 0;
  for (const novel of novels) {
    if (!Number.isFinite(novel.diskBytes) || novel.diskBytes <= 0) continue;
    if (!Number.isFinite(novel.downloadedChapters) || novel.downloadedChapters <= 0) {
      continue;
    }
    bytes += novel.diskBytes;
    chapters += novel.downloadedChapters;
  }
  if (chapters <= 0) return null;
  const average = Math.round(bytes / chapters);
  return Math.min(Math.max(average, 4 * 1024), 200 * 1024);
}

export function estimateDownloadBytes(
  chapters: number,
  novels: LibraryNovel[] = [],
): SizeEstimate {
  const measured = averageChapterBytes(novels);
  const bytesPerChapter = measured ?? FALLBACK_BYTES_PER_CHAPTER;
  return {
    bytes: Math.round(Math.max(chapters, 0) * bytesPerChapter),
    bytesPerChapter,
    basis: measured === null ? "fallback" : "library",
  };
}

export function shouldWarnLargeDownload(
  allChapters: boolean,
  chapters: number | null | undefined,
): chapters is number {
  return (
    allChapters &&
    typeof chapters === "number" &&
    Number.isFinite(chapters) &&
    chapters >= LARGE_DOWNLOAD_CHAPTERS
  );
}

export type QuotaLevel = "none" | "near" | "over";

const BYTES_PER_GB = 1024 ** 3;

export function bytesToGigabytes(bytes: number): number {
  if (!Number.isFinite(bytes) || bytes <= 0) return 0;
  return Math.round((bytes / BYTES_PER_GB) * 10) / 10;
}

export function gigabytesToBytes(gigabytes: number): number {
  if (!Number.isFinite(gigabytes) || gigabytes <= 0) return 0;
  return Math.round(gigabytes * BYTES_PER_GB);
}
export function quotaLevel(used: number, quota: number): QuotaLevel {
  if (!Number.isFinite(quota) || quota <= 0) return "none";
  if (used >= quota) return "over";
  if (used >= quota * 0.9) return "near";
  return "none";
}
