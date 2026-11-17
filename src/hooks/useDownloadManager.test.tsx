import { render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useDownloadManager } from "@/hooks/useDownloadManager";
import { DownloadsPage } from "@/pages/DownloadsPage";
import { api } from "@/lib/tauri";
import type { DownloadTask } from "@/types";

vi.mock("@/lib/tauri", () => ({
  api: {
    getTasks: vi.fn(),
    checkTools: vi.fn(),
    getSettings: vi.fn(),
    listLibrary: vi.fn(),
    getAppInfo: vi.fn(),
    onLog: vi.fn(async () => () => {}),
    onProgress: vi.fn(async () => () => {}),
    onStatus: vi.fn(async () => () => {}),
    onLibraryChanged: vi.fn(async () => () => {}),
    onQuitRequested: vi.fn(async () => () => {}),
    resumeInterrupted: vi.fn(),
  },
}));

function task(overrides: Partial<DownloadTask>): DownloadTask {
  return {
    id: "task",
    title: "Untitled",
    query: "untitled",
    engine: "webnovel",
    status: "queued",
    phase: "resolving",
    createdAt: 1,
    destination: "/downloads",
    progress: 0,
    bytes: 0,
    speedBps: 0,
    etaSeconds: null,
    elapsedSeconds: 0,
    idleSeconds: 0,
    message: null,
    files: [],
    ...overrides,
  };
}

function Harness() {
  const manager = useDownloadManager();
  if (manager.loading) return <div>loading</div>;
  return <DownloadsPage manager={manager} />;
}

beforeEach(() => {
  vi.mocked(api.getTasks).mockResolvedValue([
    task({
      id: "persisted-paused",
      title: "Reverend Insanity",
      status: "paused",
      phase: "downloadingChapters",
      progress: 42,
      bytes: 12_000,
      elapsedSeconds: 90,
      idleSeconds: 4,
      createdAt: 2,
    }),
    task({
      id: "persisted-completed",
      title: "The Tell-Tale Heart (1843)",
      status: "completed",
      phase: "completed",
      progress: 100,
      bytes: 3_000,
      message: "Verified",
      elapsedSeconds: 30,
      createdAt: 1,
    }),
  ]);
  vi.mocked(api.checkTools).mockResolvedValue(null as never);
  vi.mocked(api.getSettings).mockResolvedValue(null as never);
  vi.mocked(api.listLibrary).mockResolvedValue({
    entries: [],
    stats: { count: 0, totalBytes: 0, byKind: {} },
    novels: [],
    archives: [],
  });
  vi.mocked(api.getAppInfo).mockResolvedValue(null as never);
});

describe("download task hydration after an app restart", () => {
  it("renders the persisted task list with its stored status and progress", async () => {
    render(<Harness />);

    await screen.findAllByText("Reverend Insanity");
    const transfers = screen
      .getByRole("heading", { name: "Transfers" })
      .closest("section") as HTMLElement;
    const rows = within(transfers);

    expect(rows.getByText("Reverend Insanity")).toBeInTheDocument();
    expect(rows.getByText("The Tell-Tale Heart (1843)")).toBeInTheDocument();
    expect(rows.getByText("2 total")).toBeInTheDocument();

    // Progress comes from disk, not from a fabricated animation.
    expect(
      rows.getAllByRole("progressbar").map((bar) => bar.getAttribute("aria-valuenow")),
    ).toEqual(["42", "100"]);
    expect(rows.getByText("42%")).toBeInTheDocument();
    expect(rows.getByText("100%")).toBeInTheDocument();

    expect(rows.getByText("paused")).toBeInTheDocument();
    expect(rows.getByText("Verified")).toBeInTheDocument();
    expect(vi.mocked(api.getTasks)).toHaveBeenCalledTimes(1);
  });

  it("offers a resume action for a job interrupted by a restart", async () => {
    vi.mocked(api.getTasks).mockResolvedValue([
      task({
        id: "interrupted-1",
        title: "Reverend Insanity",
        status: "interrupted",
        phase: "downloadingChapters",
        progress: 61,
        message: "NovelForge closed while this download was running. Resume to continue.",
      }),
    ]);

    render(<Harness />);

    await screen.findAllByText("Reverend Insanity");
    expect(
      screen.getByText(/Resume to continue/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Resume interrupted download" }),
    ).toBeInTheDocument();
  });
});
