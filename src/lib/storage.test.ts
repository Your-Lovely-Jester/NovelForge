import { describe, expect, it } from "vitest";
import {
  FALLBACK_BYTES_PER_CHAPTER,
  LARGE_DOWNLOAD_CHAPTERS,
  averageChapterBytes,
  estimateDownloadBytes,
  quotaLevel,
  shouldWarnLargeDownload,
} from "@/lib/storage";
import { makeNovel as novel } from "@/test/factories";

describe("download size estimate", () => {
  it("uses the library average when downloaded folders exist", () => {
    const novels = [
      novel({ downloadedChapters: 100, diskBytes: 2_000_000 }),
      novel({ downloadedChapters: 100, diskBytes: 4_000_000 }),
    ];
    expect(averageChapterBytes(novels)).toBe(30_000);
    const estimate = estimateDownloadBytes(500, novels);
    expect(estimate.bytes).toBe(15_000_000);
    expect(estimate.basis).toBe("library");
    expect(estimate.bytesPerChapter).toBe(30_000);
  });

  it("ignores empty folders and clamps implausible averages", () => {
    expect(averageChapterBytes([novel({ downloadedChapters: 0, diskBytes: 0 })])).toBeNull();
    expect(
      averageChapterBytes([novel({ downloadedChapters: 1, diskBytes: 1 })]),
    ).toBe(4 * 1024);
    expect(
      averageChapterBytes([novel({ downloadedChapters: 1, diskBytes: 500 * 1024 * 1024 })]),
    ).toBe(200 * 1024);
  });

  it("falls back to a documented constant with an empty library", () => {
    const estimate = estimateDownloadBytes(100, []);
    expect(estimate.basis).toBe("fallback");
    expect(estimate.bytesPerChapter).toBe(FALLBACK_BYTES_PER_CHAPTER);
    expect(estimate.bytes).toBe(100 * FALLBACK_BYTES_PER_CHAPTER);
  });

  it("only warns for all-chapter runs above the threshold", () => {
    expect(shouldWarnLargeDownload(true, LARGE_DOWNLOAD_CHAPTERS)).toBe(true);
    expect(shouldWarnLargeDownload(true, LARGE_DOWNLOAD_CHAPTERS - 1)).toBe(false);
    expect(shouldWarnLargeDownload(false, 2000)).toBe(false);
    expect(shouldWarnLargeDownload(true, null)).toBe(false);
  });

  it("reports near and over quota states", () => {
    expect(quotaLevel(100, 0)).toBe("none");
    expect(quotaLevel(50, 100)).toBe("none");
    expect(quotaLevel(90, 100)).toBe("near");
    expect(quotaLevel(100, 100)).toBe("over");
    expect(quotaLevel(120, 100)).toBe("over");
  });
});
