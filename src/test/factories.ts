import type { LibraryNovel } from "@/types";

export function makeNovel(overrides: Partial<LibraryNovel> = {}): LibraryNovel {
  return {
    folder: "folder",
    title: "Untitled",
    author: null,
    synopsis: null,
    tags: [],
    cover: null,
    chapterCount: 10,
    downloadedChapters: 0,
    complete: true,
    sourceUrl: null,
    modified: 1,
    epubPath: null,
    archivePath: null,
    retryable: false,
    provenance: null,
    diskBytes: 0,
    sourceHost: null,
    sourceRank: 0,
    crawl: null,
    ...overrides,
  };
}
