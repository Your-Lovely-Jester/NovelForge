import type { ResolvedTitle } from "@/types";

/** Matches below this are suggestions: the studio asks the user to confirm
 * before starting, and the automatic title-download path refuses them. */
export const CONFIRM_CONFIDENCE = 0.85;

export function confidencePercent(confidence: number): number {
  if (!Number.isFinite(confidence)) return 0;
  return Math.round(Math.min(Math.max(confidence, 0), 1) * 100);
}

export function matchTypeLabel(matchedBy: string): string {
  switch (matchedBy) {
    case "url":
      return "Direct URL";
    case "alias":
      return "Alias match";
    case "author":
      return "Author match";
    default:
      return "Title match";
  }
}

export function needsConfirmation(match: ResolvedTitle): boolean {
  return !Number.isFinite(match.confidence) || match.confidence < CONFIRM_CONFIDENCE;
}

export function confidenceTone(
  match: ResolvedTitle,
): "direct" | "strong" | "review" {
  if (match.matchedBy === "url") return "direct";
  return needsConfirmation(match) ? "review" : "strong";
}
