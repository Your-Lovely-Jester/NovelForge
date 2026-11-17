import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NovelSearchPanel } from "@/components/novels/NovelSearchPanel";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import { api } from "@/lib/tauri";
import type { CatalogueSource, SearchResponse } from "@/types";

vi.mock("@/lib/tauri", () => ({
  api: {
    listSourceCatalogue: vi.fn(),
    fetchNovelMetadata: vi.fn(),
    fetchNovelChapters: vi.fn(),
    findExistingDownload: vi.fn(),
  },
}));

function source(overrides: Partial<CatalogueSource>): CatalogueSource {
  return {
    url: "https://novelfire.net",
    host: "novelfire.net",
    language: "English",
    originLanguages: ["zh"],
    translatedFromKorean: false,
    availabilityNote: null,
    search: true,
    manga: false,
    mtl: false,
    novelforgeId: "novelfire",
    supportLevel: "verified",
    downloadReady: true,
    ...overrides,
  };
}

function manager(searchNovels: ReturnType<typeof vi.fn>): DownloadManager {
  return {
    library: {
      entries: [],
      stats: { count: 0, totalBytes: 0, byKind: {} },
      novels: [],
      archives: [],
      storage: undefined,
    },
    starting: false,
    searchNovels,
    resolveTitle: vi.fn(),
    start: vi.fn(),
    showNotice: vi.fn(),
  } as unknown as DownloadManager;
}

function search(query: string) {
  fireEvent.change(screen.getByLabelText("Search webnovels"), {
    target: { value: query },
  });
  fireEvent.click(screen.getByRole("button", { name: "Search" }));
}

beforeEach(() => {
  vi.mocked(api.listSourceCatalogue).mockResolvedValue([]);
  vi.mocked(api.fetchNovelMetadata).mockRejectedValue(
    new Error("metadata request blocked in test"),
  );
  vi.mocked(api.fetchNovelChapters).mockRejectedValue(
    new Error("chapter request blocked in test"),
  );
  vi.mocked(api.findExistingDownload).mockResolvedValue(null);
});

describe("NovelSearchPanel search failures", () => {
  it("renders partial results beside the sources that failed", async () => {
    const response: SearchResponse = {
      query: "reverend insanity",
      results: [
        {
          title: "Reverend Insanity",
          url: "https://novelfire.net/book/reverend-insanity",
          source: "https://novelfire.net",
          detail: "2334 chapters",
        },
      ],
      searched: ["novelfire.net"],
      failures: [
        {
          source: "https://novelphoenix.net",
          label: "novelphoenix.net",
          reason: "request timed out after 12 s",
        },
      ],
      cached: false,
      cacheAgeSeconds: null,
    };
    const searchNovels = vi.fn(async () => response);
    render(<NovelSearchPanel manager={manager(searchNovels)} />);

    search("reverend insanity");

    expect(await screen.findByText("Reverend Insanity")).toBeInTheDocument();
    expect(screen.getByText("Live results")).toBeInTheDocument();
    expect(screen.getByText("1 source failed")).toBeInTheDocument();
    expect(screen.getByText("novelphoenix.net")).toBeInTheDocument();
    expect(
      screen.getByText(/request timed out after 12 s/),
    ).toBeInTheDocument();
    expect(searchNovels).toHaveBeenCalledWith("reverend insanity", []);

    // A failed metadata fetch must not hide the title it did return.
    expect(
      await screen.findByText(/Metadata source blocked the request/),
    ).toBeInTheDocument();
  });

  it("lists every failed source when the whole search returns nothing", async () => {
    const response: SearchResponse = {
      query: "reverend insanity",
      results: [],
      searched: [],
      failures: [
        {
          source: "https://novelfire.net",
          label: "novelfire.net",
          reason: "HTTP 503 from the catalogue",
        },
        {
          source: "https://novelphoenix.net",
          label: "novelphoenix.net",
          reason: "page layout did not match",
        },
      ],
      cached: false,
      cacheAgeSeconds: null,
    };
    render(
      <NovelSearchPanel manager={manager(vi.fn(async () => response))} />,
    );

    search("reverend insanity");

    expect(await screen.findByText("2 sources failed")).toBeInTheDocument();
    expect(
      screen.getByText(/HTTP 503 from the catalogue/),
    ).toBeInTheDocument();
    expect(screen.getByText(/page layout did not match/)).toBeInTheDocument();
    expect(screen.getByText("No matches in this view")).toBeInTheDocument();
    expect(
      screen.getByText(/Catalogues differ/),
    ).toBeInTheDocument();
  });

  it("explains a zero-hit response without inventing rows", async () => {
    const response: SearchResponse = {
      query: "reverend insanity",
      results: [],
      searched: ["novelfire.net"],
      failures: [],
      cached: false,
      cacheAgeSeconds: null,
    };
    render(
      <NovelSearchPanel manager={manager(vi.fn(async () => response))} />,
    );

    search("reverend insanity");

    expect(await screen.findByText("No matches in this view")).toBeInTheDocument();
    expect(
      screen.getByText(/No hit from NovelForge sources/),
    ).toBeInTheDocument();
    expect(screen.queryByText("Reverend Insanity")).toBeNull();
  });

  it("refuses a query shorter than two characters before searching", () => {
    const searchNovels = vi.fn();
    const stub = manager(searchNovels);
    render(<NovelSearchPanel manager={stub} />);

    search("a");

    expect(stub.showNotice).toHaveBeenCalledWith(
      "error",
      "Search needs a title",
      "Type at least two characters.",
    );
    expect(searchNovels).not.toHaveBeenCalled();
  });

  it("blocks title search on a direct-URL-only source", async () => {
    const direct = source({
      url: "https://example.com",
      host: "example.com",
      novelforgeId: "example",
      search: false,
      downloadReady: false,
    });
    vi.mocked(api.listSourceCatalogue).mockResolvedValue([
      source({}),
      direct,
    ]);
    const searchNovels = vi.fn();
    const stub = manager(searchNovels);
    render(<NovelSearchPanel manager={stub} />);

    await screen.findByRole("option", { name: /example\.com/ });
    fireEvent.change(
      screen.getByLabelText("Choose which source to search"),
      { target: { value: direct.url } },
    );

    search("reverend insanity");

    expect(stub.showNotice).toHaveBeenCalledWith(
      "error",
      "This source is direct-URL only",
      "Paste a page link from this source instead of searching its catalogue.",
    );
    expect(searchNovels).not.toHaveBeenCalled();
  });

  it("labels a cached search response with its age", async () => {
    const response: SearchResponse = {
      query: "reverend insanity",
      results: [
        {
          title: "Reverend Insanity",
          url: "https://novelfire.net/book/reverend-insanity",
          source: "https://novelfire.net",
          detail: "2334 chapters",
        },
      ],
      searched: ["novelfire.net"],
      failures: [],
      cached: true,
      cacheAgeSeconds: 120,
    };
    render(
      <NovelSearchPanel manager={manager(vi.fn(async () => response))} />,
    );

    search("reverend insanity");

    expect(await screen.findByText(/cached 2m ago/)).toBeInTheDocument();
  });
});
