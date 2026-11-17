import { describe, expect, it } from "vitest";
import type { ResolvedTitle } from "@/types";
import {
  CONFIRM_CONFIDENCE,
  confidencePercent,
  confidenceTone,
  matchTypeLabel,
  needsConfirmation,
} from "@/lib/resolver";

function match(overrides: Partial<ResolvedTitle> = {}): ResolvedTitle {
  return {
    input: "reverend insanity",
    title: "Reverend Insanity",
    source: "https://novelphoenix.net/reverend-insanity.html",
    detail: "Matched on novelphoenix.net",
    confidence: 1,
    matchedBy: "title",
    searched: ["novelphoenix.net"],
    alternatives: [],
    cached: false,
    cacheAgeSeconds: null,
    ...overrides,
  };
}

describe("resolver confidence", () => {
  it("rounds and clamps percentages", () => {
    expect(confidencePercent(0.856)).toBe(86);
    expect(confidencePercent(1.4)).toBe(100);
    expect(confidencePercent(-1)).toBe(0);
    expect(confidencePercent(Number.NaN)).toBe(0);
  });

  it("requires confirmation below the shared threshold", () => {
    expect(needsConfirmation(match({ confidence: CONFIRM_CONFIDENCE }))).toBe(false);
    expect(needsConfirmation(match({ confidence: CONFIRM_CONFIDENCE - 0.01 }))).toBe(true);
    expect(needsConfirmation(match({ confidence: Number.NaN }))).toBe(true);
  });

  it("labels the match type and tone", () => {
    expect(matchTypeLabel("alias")).toBe("Alias match");
    expect(matchTypeLabel("author")).toBe("Author match");
    expect(matchTypeLabel("url")).toBe("Direct URL");
    expect(matchTypeLabel("title")).toBe("Title match");
    expect(confidenceTone(match({ matchedBy: "url" }))).toBe("direct");
    expect(confidenceTone(match({ confidence: 0.7 }))).toBe("review");
    expect(confidenceTone(match({ confidence: 0.9 }))).toBe("strong");
  });
});
