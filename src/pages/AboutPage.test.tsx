import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AboutPage } from "@/pages/AboutPage";
import type { DownloadManager } from "@/hooks/useDownloadManager";

const { openDonation } = vi.hoisted(() => ({
  openDonation: vi.fn(async () => {}),
}));

vi.mock("@/lib/tauri", () => ({
  api: { openDonation },
}));

function manager(): DownloadManager {
  return {
    appInfo: {
      version: "0.1.5",
      commit: "abc1234",
      platform: "linux",
      arch: "x86_64",
      tauriVersion: "2",
      downloadDir: "/tmp/novelforge",
    },
    library: {
      entries: [],
      novels: [],
      archives: [],
      stats: { count: 0, totalBytes: 0, byKind: {} },
    },
    settings: null,
    openDownloads: vi.fn(async () => {}),
    showNotice: vi.fn(),
  } as unknown as DownloadManager;
}

describe("AboutPage support", () => {
  beforeEach(() => {
    openDonation.mockClear();
  });

  it("opens the Ko-fi page from the support panel", async () => {
    render(<AboutPage manager={manager()} />);

    fireEvent.click(screen.getByRole("button", { name: /support on ko-fi/i }));

    await waitFor(() => expect(openDonation).toHaveBeenCalledTimes(1));
  });
});
