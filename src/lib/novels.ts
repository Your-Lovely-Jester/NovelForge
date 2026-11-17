import type { NovelResult } from "@/types";

export interface NovelFacts {
  chapters: number | null;
  language: string | null;
  status: string | null;
  tags: string[];
}

const EMPTY: NovelFacts = {
  chapters: null,
  language: null,
  status: null,
  tags: [],
};

/**
 * The crawler's search output packs language, tags, status and chapter count
 * into one detail line. Parse it defensively and never invent missing facts.
 */
export function novelFacts(result: NovelResult): NovelFacts {
  const raw = result.detail?.trim();
  if (!raw || raw === "|") return EMPTY;

  const chaptersMatch = raw.match(/(\d[\d,]*)\s*chapters?/i);
  const statusMatch = raw.match(/\b(full|ongoing|completed|hiatus)\b/i);
  const languageMatch = raw.match(/\b(chinese|english|korean|japanese|multi)\b/i);

  let rest = raw;
  if (chaptersMatch) rest = rest.replace(chaptersMatch[0], " ");
  if (statusMatch) rest = rest.replace(statusMatch[0], " ");
  if (languageMatch) {
    rest = rest.replace(new RegExp(`${languageMatch[0]}\\s*novel`, "i"), " ");
  }

  const tags = rest
    .split(/[,/|·]/)
    .map((tag) => tag.replace(/\bnovel\b/gi, "").trim())
    .filter((tag) => tag.length > 0 && tag.length < 32);

  const chapters = chaptersMatch
    ? Number(chaptersMatch[1].replace(/,/g, ""))
    : null;

  return {
    chapters: Number.isFinite(chapters) ? chapters : null,
    language: languageMatch ? titleCase(languageMatch[1]) : null,
    status: statusMatch ? titleCase(statusMatch[1]) : null,
    tags: [...new Set(tags)],
  };
}

export function hostLabel(source: string): string {
  const trimmed = source.trim();
  try {
    return new URL(trimmed).hostname.replace(/^www\./, "");
  } catch {
    return trimmed.replace(/^www\./, "");
  }
}

export function resultPath(url: string): string {
  try {
    const parsed = new URL(url);
    return decodeURIComponent(parsed.pathname.replace(/\/$/, ""));
  } catch {
    return url;
  }
}

function titleCase(value: string): string {
  return value.charAt(0).toUpperCase() + value.slice(1).toLowerCase();
}
