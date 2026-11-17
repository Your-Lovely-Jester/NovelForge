/**
 * Chapter selections use the same syntax as the patched crawler:
 * `all`, single numbers, ranges and comma lists. The UI normalises the
 * string locally so the user sees exactly what will be crawled.
 */
export interface ChapterSelection {
  all: boolean;
  valid: boolean;
  normalized: string | null;
  count: number | null;
  error: string | null;
}

const MAX_CHAPTERS = 20_000;

export function parseChapterSpec(input: string): ChapterSelection {
  const raw = input.trim();
  if (!raw || /^all$/i.test(raw)) {
    return { all: true, valid: true, normalized: "all", count: null, error: null };
  }

  const numbers: number[] = [];
  for (const token of raw.split(",")) {
    const part = token.trim();
    if (!part) continue;

    if (part.includes("-")) {
      const [startRaw, endRaw, ...rest] = part.split("-");
      if (rest.length > 0) {
        return invalid(`“${part}” is not a chapter range.`);
      }
      const start = parseNumber(startRaw);
      const end = parseNumber(endRaw);
      if (start === null || end === null) {
        return invalid(`“${part}” is not a chapter range.`);
      }
      const low = Math.min(start, end);
      const high = Math.max(start, end);
      if (high - low > MAX_CHAPTERS) {
        return invalid("That range is too large.");
      }
      for (let value = low; value <= high; value += 1) numbers.push(value);
    } else {
      const value = parseNumber(part);
      if (value === null) return invalid(`“${part}” is not a chapter number.`);
      numbers.push(value);
    }

    if (numbers.length > MAX_CHAPTERS) {
      return invalid(`Select at most ${MAX_CHAPTERS.toLocaleString()} chapters.`);
    }
  }

  if (numbers.length === 0) {
    return { all: true, valid: true, normalized: "all", count: null, error: null };
  }

  numbers.sort((left, right) => left - right);
  const unique = numbers.filter((value, index) => index === 0 || value !== numbers[index - 1]);

  const parts: string[] = [];
  for (let index = 0; index < unique.length; ) {
    const start = unique[index];
    let end = start;
    while (index + 1 < unique.length && unique[index + 1] === end + 1) {
      index += 1;
      end = unique[index];
    }
    parts.push(start === end ? String(start) : `${start}-${end}`);
    index += 1;
  }

  return {
    all: false,
    valid: true,
    normalized: parts.join(","),
    count: unique.length,
    error: null,
  };
}

function parseNumber(value: string): number | null {
  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const parsed = Number(trimmed);
  // Chapter 0 is a real chapter on sources whose catalogue starts at a
  // prologue (NovelPhoenix's ORV). A source that starts at 1 rejects 0 during
  // the download's index check instead of silently fetching the wrong row.
  return Number.isSafeInteger(parsed) ? parsed : null;
}

function invalid(error: string): ChapterSelection {
  return { all: false, valid: false, normalized: null, count: null, error };
}

export function chapterHint(selection: ChapterSelection): string {
  if (!selection.valid) return selection.error ?? "Invalid chapter selection.";
  if (selection.all) return "Every chapter, in reading order.";
  if (selection.count === 1) return "1 chapter selected.";
  return `${selection.count?.toLocaleString()} chapters selected.`;
}
