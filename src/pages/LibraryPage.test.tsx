import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { LibraryPage } from "@/pages/LibraryPage";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import type {
  DownloadDetails,
  LibraryArchive,
  LibraryNovel,
  LibraryStorage,
} from "@/types";
import { makeNovel as novel } from "@/test/factories";

const storage: LibraryStorage = {
  libraryBytes: 0,
  partialBytes: 0,
  partialFolders: 0,
  workBytes: 0,
  workFolders: 0,
  quotaBytes: 0,
  overQuota: false,
};

function manager(
  novels: LibraryNovel[],
  archives: LibraryArchive[] = [],
  details: Record<string, DownloadDetails> = {},
): DownloadManager {
  const entries = novels.map((item) => ({
    name: `${item.folder}.epub`,
    path: `/library/${item.folder}/${item.folder}.epub`,
    size: item.diskBytes,
    modified: item.modified,
    kind: "epub",
    folder: item.folder,
  }));
  return {
    library: {
      entries,
      stats: {
        count: entries.length,
        totalBytes: entries.reduce((total, entry) => total + entry.size, 0),
        byKind: entries.length > 0 ? { epub: entries.length } : {},
      },
      novels,
      archives,
      storage,
    },
    loading: false,
    verify: vi.fn(async () => null),
    reveal: vi.fn(async () => {}),
    openChapters: vi.fn(async () => {}),
    deleteNovel: vi.fn(async () => true),
    retryNovel: vi.fn(async () => true),
    updateNovel: vi.fn(async () => true),
    getNovelDetails: vi.fn(
      async (folder: string) => details[folder] ?? null,
    ),
    adoptArchive: vi.fn(async () => null),
    refreshLibrary: vi.fn(async () => {}),
    openDownloads: vi.fn(async () => {}),
    cleanupEngineWork: vi.fn(async () => ({
      removed: [],
      kept: [],
      errors: [],
      bytesFreed: 0,
    })),
    deletePartialDownloads: vi.fn(async () => ({
      removed: [],
      kept: [],
      errors: [],
      bytesFreed: 0,
    })),
    findFasterSource: vi.fn(async () => null),
    startAlternative: vi.fn(async () => true),
    showNotice: vi.fn(),
  } as unknown as DownloadManager;
}

function card(title: string): HTMLElement {
  const heading = screen
    .getAllByRole("heading", { name: title })
    .find((node) => node.tagName === "H3");
  if (!heading) throw new Error(`No novel card rendered for ${title}`);
  return heading.closest("article") as HTMLElement;
}

const insanity = novel({
  folder: "reverend-insanity",
  title: "Reverend Insanity",
  author: "Gu Zhen Ren",
  chapterCount: 2334,
  downloadedChapters: 2140,
  complete: false,
  retryable: true,
  sourceUrl: "https://novelfire.net/book/reverend-insanity",
  epubPath: "/library/reverend-insanity/reverend-insanity.epub",
  modified: 1_700_000_000,
  diskBytes: 24_000_000,
});

const bloodcore = novel({
  folder: "infinite-bloodcore",
  title: "Infinite Bloodcore",
  author: "Gu Zhen Ren",
  chapterCount: 900,
  downloadedChapters: 850,
  complete: false,
  retryable: true,
  sourceUrl: "https://novelfire.net/book/infinite-bloodcore",
  epubPath: "/library/infinite-bloodcore/infinite-bloodcore.epub",
  modified: 1_700_000_100,
  diskBytes: 12_000_000,
});

const tellTale = novel({
  folder: "the-tell-tale-heart",
  title: "The Tell-Tale Heart (1843)",
  author: "Edgar Allan Poe",
  chapterCount: 1,
  downloadedChapters: 1,
  complete: true,
  retryable: false,
  provenance: "migrated",
  epubPath: "/library/the-tell-tale-heart/the-tell-tale-heart.epub",
  modified: 1_700_000_200,
  diskBytes: 500_000,
});

const looseArchive: LibraryArchive = {
  name: "loose-novel.epub",
  path: "/library/loose-novel.epub",
  folder: "library",
  kind: "epub",
  size: 512_000,
  modified: 1_700_000_300,
  adoptable: true,
  reason: "Ready to adopt",
};

describe("LibraryPage novel state", () => {
  it("shows the incomplete warning and hides retry/update for migrated entries", () => {
    render(<LibraryPage manager={manager([insanity, tellTale])} />);

    expect(screen.getByText("1 incomplete")).toBeInTheDocument();

    const migrated = within(card("The Tell-Tale Heart (1843)"));
    expect(migrated.getByText("Migrated")).toBeInTheDocument();
    expect(migrated.queryByRole("button", { name: /retry missing/i })).toBeNull();
    expect(migrated.queryByRole("button", { name: /^update$/i })).toBeNull();

    const incomplete = within(card("Reverend Insanity"));
    expect(
      incomplete.getByText(/Partial output is kept for inspection/),
    ).toBeInTheDocument();
    expect(
      incomplete.getByRole("button", { name: /retry missing/i }),
    ).toBeInTheDocument();
  });

  it("opens the incomplete-index warning with the listed-chapter detail", async () => {
    const details: Record<string, DownloadDetails> = {
      "reverend-insanity": {
        folder: "reverend-insanity",
        title: "Reverend Insanity",
        sourceUrl: "https://novelfire.net/book/reverend-insanity",
        chapterCount: 2334,
        downloadedChapters: 2140,
        selection: null,
        indexComplete: false,
        indexDetail: "2,140 of 2,334 chapters listed",
        complete: false,
        chapters: [
          {
            number: 1,
            title: "The Demon Venerable",
            url: "https://novelfire.net/book/reverend-insanity/chapter-1",
            volume: null,
            downloaded: true,
            file: "chapter-0001.xhtml",
          },
        ],
      },
      "the-tell-tale-heart": {
        folder: "the-tell-tale-heart",
        title: "The Tell-Tale Heart (1843)",
        sourceUrl: null,
        chapterCount: 1,
        downloadedChapters: 1,
        selection: null,
        indexComplete: false,
        indexDetail: "1 chapter in the EPUB spine",
        complete: true,
        chapters: [
          {
            number: 1,
            title: "The Tell-Tale Heart",
            url: "",
            volume: null,
            downloaded: true,
            file: "chapter-0001.xhtml",
          },
        ],
      },
    };
    render(<LibraryPage manager={manager([insanity, tellTale], [], details)} />);

    fireEvent.click(
      within(card("Reverend Insanity")).getByRole("button", { name: "Index" }),
    );

    expect(await screen.findByText("incomplete index")).toBeInTheDocument();
    expect(
      screen.getByText(/2,140 of 2,334 chapters listed/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/does not prove it covers the whole source/),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Close index" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    fireEvent.click(
      within(card("The Tell-Tale Heart (1843)")).getByRole("button", {
        name: "Index",
      }),
    );

    expect(await screen.findByText("local EPUB index")).toBeInTheDocument();
    expect(
      screen.getByText(/Chapter order follows the EPUB spine/),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/does not prove it covers the whole source/),
    ).toBeNull();
  });
});

describe("LibraryPage actions use the clicked folder", () => {
  it("retries the second card's folder, not the first", async () => {
    const stub = manager([insanity, bloodcore]);
    render(<LibraryPage manager={stub} />);

    fireEvent.click(
      within(card("Infinite Bloodcore")).getByRole("button", {
        name: /retry missing/i,
      }),
    );

    await waitFor(() =>
      expect(stub.retryNovel).toHaveBeenCalledWith("infinite-bloodcore"),
    );
    expect(stub.retryNovel).toHaveBeenCalledTimes(1);
  });

  it("updates the second complete card's folder", async () => {
    const stub = manager([
      { ...insanity, complete: true, downloadedChapters: 2334 },
      { ...bloodcore, complete: true, downloadedChapters: 900 },
    ]);
    render(<LibraryPage manager={stub} />);

    fireEvent.click(
      within(card("Infinite Bloodcore")).getByRole("button", {
        name: /^update$/i,
      }),
    );

    await waitFor(() =>
      expect(stub.updateNovel).toHaveBeenCalledWith("infinite-bloodcore"),
    );
    expect(stub.updateNovel).toHaveBeenCalledTimes(1);
  });

  it("deletes the folder named in the confirmation dialog", async () => {
    const stub = manager([insanity, bloodcore]);
    render(<LibraryPage manager={stub} />);

    fireEvent.click(
      within(card("Infinite Bloodcore")).getByRole("button", {
        name: "Delete Infinite Bloodcore",
      }),
    );

    expect(
      screen.getByRole("heading", { name: "Delete this download?" }),
    ).toBeInTheDocument();
    expect(
      within(screen.getByRole("dialog")).getByText("infinite-bloodcore"),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Delete permanently" }));

    await waitFor(() =>
      expect(stub.deleteNovel).toHaveBeenCalledWith("infinite-bloodcore"),
    );
    expect(stub.deleteNovel).toHaveBeenCalledTimes(1);
  });

  it("adopts the exact archive path that was clicked", async () => {
    const stub = manager([], [looseArchive]);
    render(<LibraryPage manager={stub} />);

    fireEvent.click(screen.getByRole("button", { name: "Adopt" }));

    await waitFor(() =>
      expect(stub.adoptArchive).toHaveBeenCalledWith("/library/loose-novel.epub"),
    );
    expect(stub.adoptArchive).toHaveBeenCalledTimes(1);
  });
});

describe("LibraryPage measured pace and faster source", () => {
  const paced = {
    ...insanity,
    sourceHost: "novelfire.net",
    sourceRank: 5,
    crawl: {
      chapters: 20,
      seconds: 120,
      secondsPerChapter: 6,
      chaptersPerMinute: 10,
      measuredAt: 1_700_000_000,
    },
  };

  it("shows the measured pace on the card", () => {
    render(<LibraryPage manager={manager([paced])} />);

    const view = within(card("Reverend Insanity"));
    expect(view.getByText(/Measured 10\.0 ch\/min/)).toBeInTheDocument();
    expect(view.getByText(/6\.0 s\/chapter/)).toBeInTheDocument();
  });

  it("offers a verified cheaper host as a new download and never switches", async () => {
    const stub = manager([paced]);
    stub.findFasterSource = vi.fn(async () => ({
      alternative: {
        title: "Reverend Insanity",
        url: "https://novelphoenix.net/reverend-insanity.html",
        host: "novelphoenix.net",
        detail: "Matched on novelphoenix.net",
        confidence: 1,
        currentHost: "novelfire.net",
        currentRank: 5,
        candidateRank: 0,
      },
      note: "Verified “Reverend Insanity” on novelphoenix.net — measured cheaper than novelfire.net. Starting it creates a new library folder; the existing entry at novelfire.net is not changed.",
      currentHost: "novelfire.net",
      currentRank: 5,
    }));
    render(<LibraryPage manager={stub} />);

    fireEvent.click(
      within(card("Reverend Insanity")).getByRole("button", {
        name: /faster source/i,
      }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Measured cheaper")).toBeInTheDocument();
    expect(
      within(dialog).getByText(/Nothing switches automatically/),
    ).toBeInTheDocument();

    fireEvent.click(
      within(dialog).getByRole("button", {
        name: "Download from novelphoenix.net",
      }),
    );

    await waitFor(() =>
      expect(stub.startAlternative).toHaveBeenCalledWith(
        "Reverend Insanity",
        "https://novelphoenix.net/reverend-insanity.html",
        "novelphoenix.net",
      ),
    );
    expect(stub.startAlternative).toHaveBeenCalledTimes(1);
    expect(stub.updateNovel).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("keeps the current source when the dialog is dismissed", async () => {
    const stub = manager([paced]);
    stub.findFasterSource = vi.fn(async () => ({
      alternative: null,
      note: "novelfire.net is already the fastest measured host.",
      currentHost: "novelfire.net",
      currentRank: 5,
    }));
    render(<LibraryPage manager={stub} />);

    fireEvent.click(
      within(card("Reverend Insanity")).getByRole("button", {
        name: /faster source/i,
      }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).queryByText("Measured cheaper")).toBeNull();
    const closeButtons = within(dialog).getAllByRole("button", {
      name: "Close",
    });
    fireEvent.click(closeButtons[closeButtons.length - 1]);

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(stub.startAlternative).not.toHaveBeenCalled();
  });
});
