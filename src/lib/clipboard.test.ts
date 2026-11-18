import { afterEach, describe, expect, it, vi } from "vitest";
import { copyText } from "@/lib/clipboard";

/** Replace `navigator.clipboard` for one test and restore it afterwards. */
function stubClipboard(writeText: (() => Promise<void>) | null) {
  const descriptor = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: writeText ? { writeText } : undefined,
  });
  return () => {
    if (descriptor) {
      Object.defineProperty(navigator, "clipboard", descriptor);
    } else {
      Reflect.deleteProperty(navigator, "clipboard");
    }
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("copyText", () => {
  it("uses the clipboard API when the WebView allows it", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    const restore = stubClipboard(writeText);
    await expect(copyText("report")).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith("report");
    restore();
  });

  it("falls back to the DOM copy command when the clipboard is denied", async () => {
    const restore = stubClipboard(() => Promise.reject(new Error("denied")));
    const execCommand = vi.fn().mockReturnValue(true);
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: execCommand,
    });

    await expect(copyText("report")).resolves.toBe(true);
    expect(execCommand).toHaveBeenCalledWith("copy");
    // The temporary textarea must not stay in the document.
    expect(document.querySelector("textarea")).toBeNull();
    restore();
  });

  it("reports failure when neither path works", async () => {
    const restore = stubClipboard(null);
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: vi.fn().mockReturnValue(false),
    });

    await expect(copyText("report")).resolves.toBe(false);
    expect(document.querySelector("textarea")).toBeNull();
    restore();
  });
});
