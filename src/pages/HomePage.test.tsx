import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { HomePage } from "@/pages/HomePage";
import type { DownloadManager } from "@/hooks/useDownloadManager";
import type { LibraryNovel } from "@/types";
import { makeNovel as novel } from "@/test/factories";

const tellTale = novel({
  folder: "tell-tale",
  title: "The Tell-Tale Heart (1843)",
  author: "Edgar Allan Poe",
  chapterCount: 1,
  downloadedChapters: 1,
  modified: 2,
});
const reverend = novel({
  folder: "reverend-insanity",
  title: "Reverend Insanity",
  author: "Gu Zhen Ren",
  chapterCount: 2334,
  downloadedChapters: 3,
  sourceUrl: "https://novelfire.net/book/reverend-insanity",
  modified: 1,
});

function manager(novels: LibraryNovel[]): DownloadManager {
  return {
    library: {
      entries: [],
      stats: { count: novels.length, totalBytes: 0, byKind: {} },
      novels,
      archives: [],
    },
    loading: false,
    reveal: vi.fn(async () => {}),
    showNotice: vi.fn(),
  } as unknown as DownloadManager;
}

function renderHome(novels: LibraryNovel[]) {
  const onSearch = vi.fn();
  const onNavigate = vi.fn();
  const result = render(
    <HomePage
      manager={manager(novels)}
      enginesReady={2}
      enginesTotal={3}
      onNavigate={onNavigate}
      onSearch={onSearch}
    />,
  );
  const sourceSelect = screen.getAllByRole("combobox")[0] as HTMLSelectElement;
  return { ...result, sourceSelect };
}

function optionLabels(select: HTMLSelectElement): string[] {
  return Array.from(select.options).map((option) => option.textContent ?? "");
}

function rowTitles(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll("article h3")).map(
    (heading) => heading.textContent ?? "",
  );
}

describe("HomePage source filter", () => {
  it("labels the filter as downloaded sources and shows real counts", () => {
    const { sourceSelect } = renderHome([tellTale, reverend]);

    expect(sourceSelect.value).toBe("all");
    expect(optionLabels(sourceSelect)).toEqual([
      "All downloaded sources · 2 novels",
      "Local / imported · 1 novel",
      "novelfire.net · 1 novel",
    ]);
  });

  it("filters the shelf by the selected source without changing counts", () => {
    const { container, sourceSelect } = renderHome([tellTale, reverend]);
    expect(rowTitles(container)).toEqual([
      "The Tell-Tale Heart (1843)",
      "Reverend Insanity",
    ]);

    fireEvent.change(sourceSelect, { target: { value: "novelfire.net" } });
    expect(sourceSelect.value).toBe("novelfire.net");
    expect(rowTitles(container)).toEqual(["Reverend Insanity"]);
    expect(optionLabels(sourceSelect)).toContain("novelfire.net · 1 novel");

    fireEvent.change(sourceSelect, { target: { value: "local" } });
    expect(rowTitles(container)).toEqual(["The Tell-Tale Heart (1843)"]);

    fireEvent.change(sourceSelect, { target: { value: "all" } });
    expect(rowTitles(container)).toEqual([
      "The Tell-Tale Heart (1843)",
      "Reverend Insanity",
    ]);
  });

  it("falls back to all sources when the selected source leaves the shelf", () => {
    const { container, sourceSelect, rerender } = renderHome([tellTale, reverend]);

    fireEvent.change(sourceSelect, { target: { value: "local" } });
    expect(rowTitles(container)).toEqual(["The Tell-Tale Heart (1843)"]);

    rerender(
      <HomePage
        manager={manager([reverend])}
        enginesReady={2}
        enginesTotal={3}
        onNavigate={vi.fn()}
        onSearch={vi.fn()}
      />,
    );

    const select = screen.getAllByRole("combobox")[0] as HTMLSelectElement;
    expect(select.value).toBe("all");
    expect(rowTitles(container)).toEqual(["Reverend Insanity"]);
    expect(optionLabels(select)).toEqual([
      "All downloaded sources · 1 novel",
      "novelfire.net · 1 novel",
    ]);
  });
});
