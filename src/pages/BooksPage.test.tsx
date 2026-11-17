import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { BooksPage } from "@/pages/BooksPage";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import type {
  BookCandidate,
  BookSearchResponse,
  DownloadOptions,
  Engine,
  HardcoverStatus,
  LibraryNovel,
} from "@/types";

const mocks = vi.hoisted(() => ({
  searchBooks: vi.fn(),
  getHardcoverStatus: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  isTauriRuntime: () => true,
  api: {
    searchBooks: mocks.searchBooks,
    getHardcoverStatus: mocks.getHardcoverStatus,
  },
}));

function candidate(overrides: Partial<BookCandidate>): BookCandidate {
  return {
    id: "hardcover:1",
    title: "Untitled",
    authors: ["Rebecca Yarros"],
    year: 2023,
    isbn: null,
    publisher: "Entangled",
    language: null,
    series: "The Empyrean",
    seriesPosition: 1,
    coverUrl: null,
    source: "hardcover",
    url: null,
    releaseDate: null,
    released: true,
    ...overrides,
  };
}

function novel(overrides: Partial<LibraryNovel>): LibraryNovel {
  return {
    folder: "/tmp/novelforge/Rebecca Yarros/The Empyrean/01 - Fourth Wing",
    title: "Fourth Wing",
    author: "Rebecca Yarros",
    synopsis: null,
    tags: [],
    cover: null,
    chapterCount: 46,
    downloadedChapters: 46,
    complete: true,
    sourceUrl: null,
    modified: 1,
    epubPath: "01 - Fourth Wing.epub",
    retryable: false,
    provenance: null,
    diskBytes: 1,
    sourceHost: null,
    sourceRank: 0,
    crawl: null,
    ...overrides,
  };
}

function manager(
  start: ReturnType<typeof vi.fn>,
  novels: LibraryNovel[] = [],
): DownloadManager {
  return {
    library: {
      entries: [],
      stats: { count: 0, totalBytes: 0, byKind: {} },
      novels,
      archives: [],
    },
    settings: { bookLanguage: null },
    starting: false,
    start,
    showNotice: vi.fn(),
    reveal: vi.fn(async () => {}),
    openDownloads: vi.fn(async () => {}),
  } as unknown as DownloadManager;
}

function response(overrides: Partial<BookSearchResponse>): BookSearchResponse {
  return {
    query: "The Empyrean",
    candidates: [],
    series: null,
    searched: ["hardcover"],
    failures: [],
    cached: false,
    cacheAgeSeconds: null,
    hardcoverConfigured: true,
    ...overrides,
  };
}

const status: HardcoverStatus = {
  configured: false,
  path: "/home/test/.config/novelforge/hardcover-token",
  detail: null,
};

function startMock() {
  return vi.fn(
    async (
      _engine: Engine,
      _query: string,
      _options?: DownloadOptions,
      _title?: string,
    ) => true,
  );
}

async function search(query: string) {
  fireEvent.change(screen.getByLabelText("Book or series title"), {
    target: { value: query },
  });
  fireEvent.click(screen.getByRole("button", { name: /Search catalogue/ }));
}

describe("BooksPage series picker", () => {
  beforeEach(() => {
    mocks.searchBooks.mockReset();
    mocks.getHardcoverStatus.mockReset();
    mocks.getHardcoverStatus.mockResolvedValue(status);
  });

  it("lists a series in position order and queues only released books", async () => {
    mocks.searchBooks.mockResolvedValue(
      response({
        series: {
          id: "hardcover:9",
          name: "The Empyrean",
          author: "Rebecca Yarros",
          source: "hardcover",
          books: [
            candidate({ id: "hardcover:1", title: "Fourth Wing", seriesPosition: 1 }),
            candidate({ id: "hardcover:2", title: "Iron Flame", seriesPosition: 2 }),
            candidate({
              id: "hardcover:3",
              title: "Threshing Day",
              seriesPosition: 3.5,
              releaseDate: "2026-09-29",
            }),
            candidate({
              id: "hardcover:4",
              title: "Book Four",
              seriesPosition: 4,
              releaseDate: "2999-01-01",
              released: false,
            }),
          ],
        },
      }),
    );
    const start = startMock();
    render(<BooksPage manager={manager(start)} onOpenSettings={() => undefined} />);

    await search("The Empyrean");
    expect(await screen.findByText("The Empyrean")).toBeInTheDocument();
    expect(screen.getByText("#3.5")).toBeInTheDocument();
    expect(screen.getByText("Book Four")).toBeInTheDocument();
    expect(screen.getByText(/upcoming/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Queue 3 selected/ }));

    await waitFor(() => expect(start).toHaveBeenCalledTimes(3));
    const titles = start.mock.calls.map((call) => call[3]);
    expect(titles).toEqual(["Fourth Wing", "Iron Flame", "Threshing Day"]);
    const options = start.mock.calls[0][2];
    expect(options).toMatchObject({
      bookSource: "auto",
      bookTitle: "Fourth Wing",
      bookAuthor: "Rebecca Yarros",
      bookSeries: "The Empyrean",
      bookSeriesPosition: 1,
    });
    expect(start.mock.calls.map((call) => call[1])).not.toContain("Book Four");
  });

  it("skips series books that are already complete in the library", async () => {
    mocks.searchBooks.mockResolvedValue(
      response({
        series: {
          id: "hardcover:9",
          name: "The Empyrean",
          author: "Rebecca Yarros",
          source: "hardcover",
          books: [
            candidate({ id: "hardcover:1", title: "Fourth Wing", seriesPosition: 1 }),
            candidate({ id: "hardcover:2", title: "Iron Flame", seriesPosition: 2 }),
          ],
        },
      }),
    );
    const start = startMock();
    const value = manager(start, [novel({})]);
    render(<BooksPage manager={value} onOpenSettings={() => undefined} />);

    await search("The Empyrean");
    expect(await screen.findByText("· already shelved")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Queue 2 selected/ }));

    await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
    expect(start.mock.calls[0][3]).toBe("Iron Flame");
    expect(
      (value.showNotice as unknown as ReturnType<typeof vi.fn>),
    ).toHaveBeenCalledWith(
      "success",
      "Series queue",
      expect.stringContaining("1 already complete and skipped"),
    );
  });

  it("fetches a single catalogue record with its metadata", async () => {
    mocks.searchBooks.mockResolvedValue(
      response({
        candidates: [
          candidate({
            id: "openlibrary:/works/OL1W",
            title: "Fourth Wing",
            isbn: "9781649374042",
            source: "openlibrary",
            series: "The Empyrean",
            seriesPosition: 1,
          }),
        ],
        searched: ["openlibrary"],
        hardcoverConfigured: false,
      }),
    );
    const start = startMock();
    render(<BooksPage manager={manager(start)} onOpenSettings={() => undefined} />);

    await search("Fourth Wing");
    fireEvent.click(await screen.findByRole("button", { name: /Fetch/ }));

    await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
    expect(start.mock.calls[0][1]).toBe("Fourth Wing");
    expect(start.mock.calls[0][2]).toMatchObject({
      bookTitle: "Fourth Wing",
      bookAuthor: "Rebecca Yarros",
      bookIsbn: "9781649374042",
    });
  });

  it("uses an explicitly selected public-domain source", async () => {
    mocks.searchBooks.mockResolvedValue(
      response({
        candidates: [
          candidate({
            id: "openlibrary:/works/OL2W",
            title: "Pride and Prejudice",
            authors: ["Jane Austen"],
            isbn: null,
            series: null,
            seriesPosition: null,
            source: "openlibrary",
          }),
        ],
        searched: ["openlibrary"],
        hardcoverConfigured: false,
      }),
    );
    const start = startMock();
    render(<BooksPage manager={manager(start)} onOpenSettings={() => undefined} />);

    await search("Pride and Prejudice");
    fireEvent.click(
      screen.getByRole("button", { name: /Project Gutenberg/ }),
    );
    expect(
      screen.getByText(/Opt-in public-domain catalogue/),
    ).toBeInTheDocument();

    fireEvent.click(await screen.findByRole("button", { name: /Fetch/ }));

    await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
    expect(start.mock.calls[0][2]).toMatchObject({
      bookSource: "gutenberg",
      bookTitle: "Pride and Prejudice",
      bookAuthor: "Jane Austen",
    });
  });

  it("reports failed catalogue sources instead of inventing rows", async () => {
    mocks.searchBooks.mockResolvedValue(
      response({
        searched: [],
        failures: [
          {
            source: "openlibrary",
            label: "Open Library",
            reason: "Open Library returned 503: unavailable",
          },
        ],
      }),
    );
    const start = startMock();
    render(<BooksPage manager={manager(start)} onOpenSettings={() => undefined} />);

    await search("Fourth Wing");
    expect(
      await screen.findByText(/Open Library returned 503/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/nothing is downloaded until a verified record is selected/i),
    ).toBeInTheDocument();
  });
});
