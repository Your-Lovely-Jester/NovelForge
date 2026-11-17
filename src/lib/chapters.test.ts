import { describe, expect, it } from "vitest";
import { parseChapterSpec } from "./chapters";

describe("parseChapterSpec", () => {
  it("keeps chapter 0 so a 0-based catalogue can select its prologue", () => {
    const selection = parseChapterSpec("0-2");
    expect(selection.valid).toBe(true);
    expect(selection.normalized).toBe("0-2");
    expect(selection.count).toBe(3);
  });

  it("still rejects negative and non-numeric tokens", () => {
    expect(parseChapterSpec("-1-2").valid).toBe(false);
    expect(parseChapterSpec("one").valid).toBe(false);
  });

  it("sorts and compresses a mixed selection", () => {
    const selection = parseChapterSpec("5,2-3,3,0");
    expect(selection.valid).toBe(true);
    expect(selection.normalized).toBe("0,2-3,5");
    expect(selection.count).toBe(4);
  });
});
