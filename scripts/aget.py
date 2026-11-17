#!/usr/bin/env python3
"""NovelForge book downloader.

One CLI, four free sources:

    aget "reverend insanity" --auto
    aget "fourth wing" --title "Fourth Wing" --author "Rebecca Yarros" --auto
    aget "dune" --source libgen --auto

`auto` walks libgen -> welib -> Anna's Archive -> OceanOfPDF and stops at the
first EPUB that matches the expected title/author. `gutenberg` and
`standardebooks` are lawful public-domain catalogues but are never part of
`auto`; they only run when selected explicitly. Expected metadata is
mandatory for a meaningful score: without it aget can only report candidates,
never guess. Each source has a per-source budget (default 150 s, override with
--budget or NOVELFORGE_SOURCE_BUDGET); a blocked challenge becomes one visible
"… — skipped" line instead of a hung job. OceanOfPDF is last and runs a
headless browser by default; the visible window is opt-in through
NOVELFORGE_OCEAN_HEADFUL=1 for debugging only.
"""

from __future__ import annotations

import asyncio
import difflib
import html
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from html.parser import HTMLParser
from pathlib import Path
from typing import Iterable, Optional

import click
from rich.console import Console

console = Console()

ANNAS_URL = "https://annas-archive.gl"
WELIB_URLS = ("https://welib.gl", "https://welib.st")
LIBGEN_MIRRORS = ("https://libgen.li", "https://libgen.is", "https://libgen.rs")
OCEAN_URL = "https://oceanofpdf.com"
USER_AGENT = (
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
)
# `auto` is exactly this order; the public-domain sources below are opt-in.
SOURCES = ("libgen", "welib", "annas", "ocean")
PUBLIC_DOMAIN_SOURCES = ("gutenberg", "standardebooks")
ALL_SOURCES = (*SOURCES, *PUBLIC_DOMAIN_SOURCES)
# A candidate below this score is a guess. Guessing is how the wrong book was
# downloaded before; the default path now fails closed instead.
MATCH_THRESHOLD = 0.62
# A row whose title barely resembles the request is not the same book, however
# well the author matches. "Dune Messiah" scores 0.50 against "Dune".
MIN_TITLE_SCORE = 0.55
# A punctuation-suffix match ("Dune: Tales of Dune …") needs the author to
# confirm it, because the title alone cannot separate an edition from a
# derivative work.
PREFIX_AUTHOR_FLOOR = 0.8
# NF-030: the budget is per source, not per attempt. A DDoS-Guard/Cloudflare
# hold must fail fast and visibly instead of consuming the whole job, so every
# polling loop checks this deadline.
SOURCE_BUDGET_SECONDS = max(
    20.0, float(os.environ.get("NOVELFORGE_SOURCE_BUDGET", "150"))
)
# A search page normally answers in ~4 s. Three dead mirrors at 20 s each used
# to consume 120+ s of a 150 s budget and leave nothing for the file transfer,
# so the per-mirror cap is tighter and 25 s is always reserved for the download.
LIBGEN_SEARCH_TIMEOUT = max(
    5.0, float(os.environ.get("NOVELFORGE_LIBGEN_TIMEOUT", "15"))
)
LIBGEN_DOWNLOAD_RESERVE = 25.0


# --------------------------------------------------------------------------- #
# Shared helpers
# --------------------------------------------------------------------------- #


def fetch(url: str, *, data: Optional[dict] = None, headers: Optional[dict] = None,
          timeout: int = 60) -> bytes:
    request_headers = {
        "User-Agent": USER_AGENT,
        "Accept": "text/html,application/xhtml+xml,application/epub+zip,*/*;q=0.8",
        "Accept-Encoding": "identity",
    }
    if headers:
        request_headers.update(headers)
    body = urllib.parse.urlencode(data).encode() if data is not None else None
    request = urllib.request.Request(url, data=body, headers=request_headers)
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read()


def remaining(deadline: float) -> float:
    return max(0.0, deadline - time.monotonic())


def skip_note(source: str, reason: str, started: float) -> None:
    """One honest line when a source is blocked, empty or out of time."""
    console.print(
        f"[yellow]{source}: {reason} after "
        f"{time.monotonic() - started:.1f}s — skipped[/yellow]"
    )


def safe_name(value: str, fallback: str = "book") -> str:
    value = re.sub(r"[^\w\s.-]", "", value, flags=re.UNICODE).strip()
    value = re.sub(r"\s+", " ", value)
    value = re.sub(r"\s+\.", ".", value)
    return (value[:90] or fallback).strip(" .")


def filename_from_url(url: str, fallback: str) -> str:
    path = urllib.parse.urlparse(url).path
    name = urllib.parse.unquote(path.rsplit("/", 1)[-1])
    return safe_name(name, fallback)


def save_bytes(payload: bytes, output_dir: str, filename: str) -> Optional[Path]:
    if len(payload) < 4096:
        console.print(f"[yellow]{filename}: response too small to be a book[/yellow]")
        return None
    output = Path(output_dir)
    output.mkdir(parents=True, exist_ok=True)
    path = output / filename
    path.write_bytes(payload)
    console.print(f"[green]DOWNLOADED:[/green] {path} ({len(payload) / 1024 / 1024:.2f} MB)")
    return path


def download_direct(url: str, output_dir: str, fallback_name: str) -> Optional[Path]:
    try:
        payload = fetch(url, timeout=240)
    except Exception as error:
        console.print(f"[yellow]Direct download failed: {error}[/yellow]")
        return None
    return save_bytes(payload, output_dir, filename_from_url(url, fallback_name))


def query_tokens(query: str) -> list[str]:
    return [token for token in re.findall(r"\w+", query.lower()) if len(token) > 2]


def looks_like_epub(blob: str) -> bool:
    return bool(re.search(r"\bepub\b", blob, re.I))


# --------------------------------------------------------------------------- #
# Metadata matching
# --------------------------------------------------------------------------- #

ARTICLES = {"the", "a", "an"}
# A single novel must never be satisfied by a collection/omnibus that happens to
# share its prefix. The measurement run picked "Dune: The Gateway Collection"
# before this guard existed.
OMNIBUS_MARKERS = (
    "collection",
    "omnibus",
    "complete series",
    "complete works",
    "box set",
    "boxset",
    "trilogy",
    "compendium",
    "anthology",
    "books 1-",
    "books 1–",
)


def normalize_text(value: str) -> str:
    words = [
        word
        for word in re.findall(r"\w+", (value or "").lower())
        if word not in ARTICLES
    ]
    return " ".join(words)


def clean_source_heading(value: str) -> str:
    """Strip site chrome from a result heading before title matching.

    OceanOfPDF headings look like `[PDF] [EPUB] Fourth Wing Download`; the
    brackets and the trailing `Download` otherwise dominate the similarity
    score and a correct result is rejected.
    """
    text = re.sub(r"\[(?:pdf|epub|mobi|azw3|txt)\]", " ", value or "", flags=re.I)
    text = re.sub(r"\b(?:free\s+)?download\b", " ", text, flags=re.I)
    text = re.sub(r"\s*[|·—–-]\s*$", " ", text)
    return re.sub(r"\s+", " ", text).strip()


def title_similarity(expected: str, actual: str) -> float:
    raw_left = (expected or "").strip().lower()
    raw_right = (actual or "").strip().lower()
    left = normalize_text(expected)
    right = normalize_text(actual)
    if not left or not right:
        return 0.0
    if left == right:
        return 1.0
    left_tokens = set(left.split())
    right_tokens = set(right.split())
    if not left_tokens or not right_tokens:
        return 0.0
    shared = len(left_tokens & right_tokens)
    union = len(left_tokens | right_tokens)
    jaccard = shared / union if union else 0.0
    ratio = difflib.SequenceMatcher(None, left, right).ratio()
    # A parenthetical/colon suffix is usually part of the same product
    # ("Fourth Wing (The Empyrean, #1)", "The Great Gatsby: The Authentic
    # Edition …"), but the same shape also hides a different book
    # ("Dune: Tales of Dune Expanded Edition: Brian Herbert & Kevin J
    # Anderson"). Credit the shape here and let `candidate_score` require a
    # confirming author for it. A bare word continuation never qualifies:
    # "Dune" must not match "Dune Messiah". Punctuation is stripped by
    # normalize_text, so inspect the raw strings.
    tail = raw_right[len(raw_left):].lstrip() if raw_right.startswith(raw_left) else ""
    prefix = 0.92 if tail[:1] in ("(", "[", ":") else 0.0
    return max(0.55 * jaccard + 0.45 * ratio, prefix)


def author_similarity(expected: str, actual: str) -> float:
    left = normalize_text(expected)
    right = normalize_text(actual)
    if not left or not right:
        return 0.0
    if left == right:
        return 1.0
    left_tokens = set(left.split())
    right_tokens = set(right.split())
    if not left_tokens or not right_tokens:
        return 0.0
    # Author lists are often "Rebecca Yarros; ..." or "Yarros, Rebecca". Divide by
    # the number of names on the row, not the number expected: a row crediting
    # three co-authors is not the same product as the one expected author
    # ("Frank Herbert; Brian Herbert & Kevin J Anderson" scored 1.0 while a raw
    # substring test was used).
    shared = len(left_tokens & right_tokens)
    return shared / max(len(right_tokens), 1)


def extract_isbn(value: str) -> list[str]:
    found = re.findall(r"\b(?:97[89]\d{10}|\d{9}[\dXx])\b", value or "")
    return [item.replace("-", "").upper() for item in found]


def is_english(language: str) -> bool:
    value = (language or "").strip().lower()
    return (
        value == "en"
        or value.startswith("english")
        or value.startswith("en-")
        or value.startswith("en_")
    )


def language_rank(expected: Optional[str], actual: str) -> float:
    actual = (actual or "").strip().lower()
    expected = (expected or "").strip().lower()
    if expected:
        if not actual:
            return 0.5
        if is_english(expected) and is_english(actual):
            return 1.0
        return 1.0 if expected in actual or actual in expected else 0.0
    if not actual:
        return 0.5
    # No language was requested. The catalogue and every user-facing source
    # path is English-first, so a row that openly declares another language is
    # a different product ("Dune 2a ed", Spanish, scored 0.65 and was picked
    # before this rule). An explicit `--lang` still overrides this.
    return 1.0 if is_english(actual) else 0.0


def candidate_score(
    *,
    expected_title: Optional[str],
    expected_author: Optional[str],
    expected_isbn: Optional[str],
    expected_language: Optional[str],
    title: str,
    author: str,
    language: str,
    isbn_blob: str,
) -> float:
    if expected_title:
        title_score = title_similarity(expected_title, title)
        raw_actual = (title or "").lower()
        raw_wanted = (expected_title or "").lower()
        if any(marker in raw_actual for marker in OMNIBUS_MARKERS) and not any(
            marker in raw_wanted for marker in OMNIBUS_MARKERS
        ):
            # An omnibus is a different product, not a near miss.
            return 0.0
        # The title has to carry the match on its own. Author and language only
        # confirm a plausible title; without this floor, "Dune Messiah" scored
        # 0.72 on an exact author match and would be downloaded for "Dune".
        if title_score < MIN_TITLE_SCORE:
            return 0.0
        tail = (
            raw_actual[len(raw_wanted):].lstrip()
            if raw_actual.startswith(raw_wanted)
            else ""
        )
        prefix_only = tail[:1] in ("(", "[", ":")
    else:
        title_score = 1.0
        prefix_only = False
    if expected_author:
        author_score = author_similarity(expected_author, author)
    else:
        author_score = 1.0
    # A punctuation-suffix title match cannot distinguish an edition from a
    # derivative work, so it needs the author to confirm it.
    if prefix_only and expected_author and author_score < PREFIX_AUTHOR_FLOOR:
        return 0.0
    lang_score = language_rank(expected_language, language)
    # A row that openly declares a different language is a different product,
    # not a near miss. Without this hard rejection a French `Fourth wing` scored
    # 0.90 on an otherwise exact title (NF-031 fixture) and would have been
    # downloaded for an English request. An explicit `--lang` overrides the
    # default English bias but still rejects a mismatch.
    declared_language = (language or "").strip()
    if declared_language:
        if expected_language:
            if lang_score == 0.0:
                return 0.0
        elif not is_english(declared_language):
            return 0.0
    # Some sources (Anna's Archive) expose a title but no author at all. Treating
    # the missing author as a zero would reject every exact title match; instead,
    # renormalise over the signals the source actually has — but only when the
    # title is exact enough to carry the match on its own.
    if expected_author and not (author or "").strip():
        if title_score < 0.9:
            return 0.0
        return (0.55 * title_score + 0.10 * lang_score) / 0.65
    score = 0.55 * title_score + 0.35 * author_score + 0.10 * lang_score
    if expected_isbn:
        wanted = re.sub(r"[^0-9Xx]", "", expected_isbn).upper()
        if wanted and wanted in extract_isbn(isbn_blob):
            score += 0.2
    return score


def rank_candidates(
    rows: list[dict],
    *,
    expected_title: Optional[str],
    expected_author: Optional[str],
    expected_isbn: Optional[str],
    expected_language: Optional[str],
    fmt: str,
) -> list[tuple[float, dict]]:
    """Score every row of the requested format, best first."""
    fmt = (fmt or "epub").lower()
    candidates = [
        row
        for row in rows
        if fmt in (row.get("format") or "").lower()
        or (row.get("title") or "").lower().endswith(f".{fmt}")
    ]
    scored = []
    for row in candidates:
        score = candidate_score(
            expected_title=expected_title,
            expected_author=expected_author,
            expected_isbn=expected_isbn,
            expected_language=expected_language,
            title=row.get("title") or "",
            author=row.get("author") or "",
            language=row.get("language") or "",
            isbn_blob=row.get("isbn") or row.get("title") or "",
        )
        scored.append((score, row))
    scored.sort(key=lambda item: item[0], reverse=True)
    return scored


def choose_candidate(
    rows: list[dict],
    *,
    source: str,
    expected_title: Optional[str],
    expected_author: Optional[str],
    expected_isbn: Optional[str],
    expected_language: Optional[str],
    fmt: str,
) -> Optional[dict]:
    """Pick the best metadata match, or nothing.

    Rows are already limited to the requested format. When expected metadata is
    present a row must clear [`MATCH_THRESHOLD`]; no fallback to `rows[0]`.
    """
    scored = rank_candidates(
        rows,
        expected_title=expected_title,
        expected_author=expected_author,
        expected_isbn=expected_isbn,
        expected_language=expected_language,
        fmt=fmt,
    )
    if not scored:
        console.print(f"[yellow]{source}: no {(fmt or 'epub').lower()} rows[/yellow]")
        return None

    has_expectation = bool(expected_title or expected_author or expected_isbn)
    best_score, best = scored[0]
    console.print(
        f"[dim]{source}: best match {best_score:.2f} · "
        f"{(best.get('title') or '?')[:80]} · {best.get('author') or 'unknown'}[/dim]"
    )
    if has_expectation and best_score < MATCH_THRESHOLD:
        console.print(
            f"[yellow]{source}: no row cleared the {MATCH_THRESHOLD:.2f} "
            f"title/author threshold[/yellow]"
        )
        for score, row in scored[:3]:
            console.print(
                f"[dim]  near miss {score:.2f} · {(row.get('title') or '?')[:70]} "
                f"· {row.get('author') or '?'} · {row.get('language') or '?'}[/dim]"
            )
        return None
    return {**best, "score": best_score}


def choose_candidates(
    rows: list[dict],
    *,
    source: str,
    expected_title: Optional[str],
    expected_author: Optional[str],
    expected_isbn: Optional[str],
    expected_language: Optional[str],
    fmt: str,
    limit: int = 3,
) -> list[dict]:
    """Every verified match, best first, so a dead file host is not fatal.

    LibGen often lists several copies of the same edition; if the first one's
    storage node stalls, the next verified copy is a legitimate substitute.
    Nothing below the threshold is ever returned.
    """
    scored = rank_candidates(
        rows,
        expected_title=expected_title,
        expected_author=expected_author,
        expected_isbn=expected_isbn,
        expected_language=expected_language,
        fmt=fmt,
    )
    if not scored:
        return []
    has_expectation = bool(expected_title or expected_author or expected_isbn)
    good = [
        {**row, "score": score}
        for score, row in scored
        if not has_expectation or score >= MATCH_THRESHOLD
    ]
    return good[:limit]


# --------------------------------------------------------------------------- #
# LibGen — free HTML search + signed get.php download
# --------------------------------------------------------------------------- #


def compact_html(value: str) -> str:
    value = re.sub(r"<br\s*/?>", " | ", value)
    value = re.sub(r"<[^>]+>", " ", value)
    return re.sub(r"\s+", " ", html.unescape(value)).strip()


class LibgenRowParser(HTMLParser):
    """Extract LibGen result rows without trusting regexes around attributes.

    LibGen puts `<br>` inside `title="..."` attributes, so a naive `<a[^>]*>`
    pattern stops inside the tag and picks the wrong anchor.
    """

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.rows: list[dict] = []
        self.current: Optional[dict] = None
        self.cell: Optional[list[str]] = None
        self.anchor: Optional[list[str]] = None
        self.in_title_cell = False

    def handle_starttag(self, tag: str, attrs: list[tuple[str, Optional[str]]]) -> None:
        values = {key: value or "" for key, value in attrs}
        if tag == "tr":
            self.current = {"cells": [], "md5": None, "edition_title": None}
        elif tag == "td" and self.current is not None:
            self.cell = []
            self.in_title_cell = not self.current["cells"]
        elif tag == "a" and self.current is not None:
            href = values.get("href", "")
            md5 = re.search(r"ads\.php\?md5=([a-f0-9]{32})", href, re.I)
            if md5:
                self.current["md5"] = md5.group(1).lower()
            if (
                self.in_title_cell
                and "edition.php" in href
                and self.current["edition_title"] is None
            ):
                self.anchor = []
        elif tag == "br" and self.cell is not None:
            self.cell.append(" | ")

    def handle_endtag(self, tag: str) -> None:
        if tag == "a" and self.anchor is not None and self.current is not None:
            title = " ".join("".join(self.anchor).split())
            if title:
                self.current["edition_title"] = title
            self.anchor = None
        elif tag == "td" and self.cell is not None and self.current is not None:
            self.current["cells"].append(" ".join("".join(self.cell).split()))
            self.cell = None
            self.in_title_cell = False
        elif tag == "tr" and self.current is not None:
            if self.current["md5"] and len(self.current["cells"]) >= 8:
                self.rows.append(self.current)
            self.current = None

    def handle_data(self, data: str) -> None:
        if self.cell is not None:
            self.cell.append(data)
        if self.anchor is not None:
            self.anchor.append(data)


def parse_libgen(page: str, base: str, fmt: str) -> list[dict]:
    parser = LibgenRowParser()
    parser.feed(page)
    rows: list[dict] = []
    for parsed in parser.rows:
        cells = parsed["cells"]
        title = parsed["edition_title"] or cells[0]
        row_format = cells[7].lower()
        if fmt and row_format != fmt.lower():
            continue
        if not title:
            continue
        rows.append(
            {
                "source": "libgen",
                "base": base,
                "md5": parsed["md5"],
                "title": title,
                "author": cells[1],
                "publisher": cells[2],
                "year": cells[3],
                "language": cells[4],
                "size": compact_html(cells[6]),
                "format": row_format,
                "isbn": "; ".join(extract_isbn(" ".join(cells))),
            }
        )
    return rows


def libgen_rows(query: str, fmt: str, deadline: Optional[float] = None) -> list[dict]:
    started = time.monotonic()
    errors = []
    for base in LIBGEN_MIRRORS:
        if deadline is not None and remaining(deadline) < 5:
            break
        url = (
            f"{base}/index.php?"
            f"{urllib.parse.urlencode({'req': query, 'res': 100})}"
        )
        timeout = LIBGEN_SEARCH_TIMEOUT
        if deadline is not None:
            if remaining(deadline) < LIBGEN_DOWNLOAD_RESERVE:
                errors.append("budget exhausted before the download")
                break
            timeout = max(
                5.0,
                min(timeout, remaining(deadline) - LIBGEN_DOWNLOAD_RESERVE),
            )
        try:
            page = fetch(url, timeout=timeout).decode("utf-8", "replace")
        except urllib.error.HTTPError as error:
            errors.append(f"{base.split('//')[-1]} HTTP {error.code}")
            continue
        except Exception as error:
            errors.append(f"{base.split('//')[-1]} {type(error).__name__}")
            continue
        rows = parse_libgen(page, base, fmt)
        if rows:
            return rows
        # The mirror answered, so this is an empty result rather than a
        # blocked mirror. Trying the remaining hosts only wastes time.
        console.print(f"[dim]LibGen: {base} answered with no {fmt} rows[/dim]")
        return []
    if errors:
        skip_note("LibGen", f"no mirror answered ({'; '.join(errors)})", started)
    return []


def libgen_download(
    query: str,
    fmt: str,
    output_dir: str,
    *,
    expected_title: Optional[str],
    expected_author: Optional[str],
    expected_isbn: Optional[str],
    expected_language: Optional[str],
) -> Optional[Path]:
    console.print("[dim]LibGen: searching…[/dim]")
    started = time.monotonic()
    deadline = started + SOURCE_BUDGET_SECONDS
    # LibGen sorts by recency, so a bare classic title can surface only
    # variations/companions. Adding the expected author moves the exact edition
    # onto the first page; the bare title stays as a fallback.
    queries = []
    if expected_author:
        queries.append(f"{query} {expected_author}")
    queries.append(query)
    rows: list[dict] = []
    search_started = time.monotonic()
    for search_query in dict.fromkeys(queries):
        rows = libgen_rows(search_query, fmt, deadline)
        if rows:
            break
        # Do not spend two full mirror timeouts on the fallback.
        if time.monotonic() - search_started > LIBGEN_SEARCH_TIMEOUT * 1.5:
            break
    if not rows:
        console.print("[yellow]LibGen: no result[/yellow]")
        return None
    console.print(f"[dim]LibGen: {len(rows)} {fmt} row(s)[/dim]")
    matches = choose_candidates(
        rows,
        source="LibGen",
        expected_title=expected_title,
        expected_author=expected_author,
        expected_isbn=expected_isbn,
        expected_language=expected_language,
        fmt=fmt,
        limit=3,
    )
    if not matches:
        # choose_candidates does not log; reuse the single-candidate reporter so
        # the near-miss list is printed once.
        choose_candidate(
            rows,
            source="LibGen",
            expected_title=expected_title,
            expected_author=expected_author,
            expected_isbn=expected_isbn,
            expected_language=expected_language,
            fmt=fmt,
        )
        return None
    best = matches[0]
    console.print(
        f"[dim]LibGen: best match {best['score']:.2f} · "
        f"{(best.get('title') or '?')[:80]} · {best.get('author') or 'unknown'}[/dim]"
    )

    for chosen in matches:
        if remaining(deadline) < 15:
            break
        downloaded = _libgen_fetch_candidate(
            chosen, fmt, output_dir, deadline, expected_title
        )
        if downloaded:
            if chosen is not best:
                console.print(
                    f"[dim]LibGen: used a second verified copy "
                    f"({chosen['score']:.2f})[/dim]"
                )
            return downloaded
    skip_note("LibGen", "file host did not deliver the EPUB", started)
    return None


def _libgen_fetch_candidate(
    chosen: dict,
    fmt: str,
    output_dir: str,
    deadline: float,
    expected_title: Optional[str],
) -> Optional[Path]:
    base = chosen["base"]
    md5 = chosen["md5"]
    ads_url = f"{base}/ads.php?md5={md5}"
    try:
        ads = fetch(
            ads_url, timeout=max(5.0, min(30.0, remaining(deadline)))
        ).decode("utf-8", "replace")
    except Exception as error:
        console.print(f"[yellow]LibGen: ads page failed ({error})[/yellow]")
        return None
    link = re.search(r'href="(get\.php\?md5=[^"]+)"', ads)
    if not link:
        console.print("[yellow]LibGen: no signed get.php link on the ads page[/yellow]")
        return None
    download_url = urllib.parse.urljoin(base + "/", html.unescape(link.group(1)))
    # One capped attempt per candidate, then the caller moves on to the next
    # verified copy. A single stalled storage node must not consume the whole
    # source budget (measured: the best Pride and Prejudice copy stalled twice).
    try:
        payload = fetch(
            download_url,
            timeout=max(15.0, min(60.0, remaining(deadline) - 5)),
        )
    except urllib.error.HTTPError as error:
        console.print(f"[yellow]LibGen: download failed ({error})[/yellow]")
        return None
    except Exception as error:
        console.print(
            f"[yellow]LibGen: download failed "
            f"({type(error).__name__}: {error})[/yellow]"
        )
        return None

    expected_magic = b"%PDF" if fmt.lower() == "pdf" else b"PK\x03\x04"
    if not payload.startswith(expected_magic):
        console.print(
            f"[yellow]LibGen: payload is not a valid {fmt.upper()} "
            f"(first bytes {payload[:8]!r})[/yellow]"
        )
        return None
    name = safe_name(expected_title or chosen["title"]) + f".{fmt.lower()}"
    return save_bytes(payload, output_dir, name)


# --------------------------------------------------------------------------- #
# WeLib — direct HTTP
# --------------------------------------------------------------------------- #


def welib_search(query: str) -> list[dict]:
    started = time.monotonic()
    errors = []
    rows: list[dict] = []
    for base in WELIB_URLS:
        url = f"{base}/search?{urllib.parse.urlencode({'q': query})}"
        try:
            page = fetch(url, timeout=30).decode("utf-8", "replace")
        except urllib.error.HTTPError as error:
            errors.append(f"{base.split('//')[-1]} HTTP {error.code}")
            continue
        except Exception as error:
            errors.append(f"{base.split('//')[-1]} {type(error).__name__}")
            continue

        lowered_page = page.lower()
        if "verify you are human" in lowered_page or "captcha" in lowered_page:
            errors.append(f"{base.split('//')[-1]} CAPTCHA")
            continue

        # Each result is a `.book-card` block. Collect every EPUB row so the
        # caller can score titles instead of taking the first card.
        blocks = re.split(r'<div class="[^"]*book-card[^"]*"', page)
        for block in blocks[1:]:
            if not looks_like_epub(block):
                continue
            match = re.search(r'href="(/auto_download/([a-f0-9]{32})/0/0)"', block)
            if not match:
                continue
            title = re.search(r"<h[23][^>]*>\s*<a[^>]*>(.*?)</a>", block, re.S)
            if not title:
                title = re.search(r'<a[^>]*title="([^"]+)"', block)
            label = (
                html.unescape(re.sub(r"<[^>]+>", "", title.group(1)).strip())
                if title
                else query
            )
            author = re.search(
                r'class="[^"]*(?:author|book-author)[^"]*"[^>]*>(.*?)<', block, re.S
            )
            author_label = (
                html.unescape(re.sub(r"<[^>]+>", "", author.group(1)).strip())
                if author
                else ""
            )
            rows.append(
                {
                    "source": "welib",
                    "base": base,
                    "md5": match.group(2),
                    "title": label or query,
                    "author": author_label,
                    "language": "",
                    "format": "epub",
                }
            )
        if rows:
            return rows
        console.print(f"[dim]WeLib: {base} answered with no EPUB cards[/dim]")
    if errors:
        skip_note("WeLib", f"blocked ({'; '.join(errors)})", started)
    elif not rows:
        skip_note("WeLib", "no EPUB result on either mirror", started)
    return rows


def welib_download(
    query: str,
    output_dir: str,
    *,
    expected_title: Optional[str],
    expected_author: Optional[str],
    expected_isbn: Optional[str],
    expected_language: Optional[str],
) -> Optional[Path]:
    console.print("[dim]WeLib: searching…[/dim]")
    rows = welib_search(query)
    if not rows:
        console.print("[yellow]WeLib: no EPUB result[/yellow]")
        return None

    chosen = choose_candidate(
        rows,
        source="WeLib",
        expected_title=expected_title,
        expected_author=expected_author,
        expected_isbn=expected_isbn,
        expected_language=expected_language,
        fmt="epub",
    )
    if not chosen:
        return None

    console.print(f"[dim]WeLib:[/dim] {chosen['title'][:90]}")
    for path in (
        f"/slow_download/{chosen['md5']}/0/0",
        f"/auto_download/{chosen['md5']}/0/0",
    ):
        try:
            page = fetch(chosen["base"] + path, timeout=60).decode("utf-8", "replace")
        except Exception as error:
            console.print(f"[yellow]WeLib: {path} failed ({error})[/yellow]")
            continue
        direct = re.search(r"navigator\.clipboard\.writeText\('([^']+)'", page)
        if direct:
            url = html.unescape(direct.group(1)).replace("\\/", "/")
            downloaded = download_direct(
                url, output_dir, safe_name(expected_title or chosen["title"]) + ".epub"
            )
            if downloaded:
                return downloaded
        # Some variants link a direct CDN file without the clipboard shim.
        direct = re.search(r'https://[^"\'<> ]+\.epub[^"\'<> ]*', page)
        if direct:
            downloaded = download_direct(
                html.unescape(direct.group(0)),
                output_dir,
                safe_name(expected_title or chosen["title"]) + ".epub",
            )
            if downloaded:
                return downloaded
    console.print("[yellow]WeLib: no direct link extracted[/yellow]")
    return None


# --------------------------------------------------------------------------- #
# Public-domain sources — Project Gutenberg and Standard Ebooks (opt-in only)
# --------------------------------------------------------------------------- #

# These two never run under `auto`. The user selects them explicitly in Books,
# because their catalogues are public-domain/old titles and are not a
# substitute for the shadow-library order. The file itself is still verified
# (EPUB magic bytes) and audited by the Rust side before it reaches the library.

GUTENBERG_URL = "https://www.gutenberg.org"
STANDARDEBOOKS_URL = "https://standardebooks.org"

LANGUAGE_NAMES = {
    "en": "English",
    "fr": "French",
    "de": "German",
    "es": "Spanish",
    "it": "Italian",
    "pt": "Portuguese",
    "nl": "Dutch",
    "ru": "Russian",
    "ja": "Japanese",
    "zh": "Chinese",
    "ko": "Korean",
}


def language_name(code: str) -> str:
    value = (code or "").strip().lower()
    if value.startswith("en"):
        return "English"
    return LANGUAGE_NAMES.get(value[:2], code.strip())


def payload_matches_format(payload: bytes, fmt: str) -> bool:
    magic = b"%PDF" if (fmt or "epub").lower() == "pdf" else b"PK\x03\x04"
    return payload.startswith(magic)


def save_verified_payload(
    payload: bytes, output_dir: str, name: str, fmt: str, source: str
) -> Optional[Path]:
    if not payload_matches_format(payload, fmt):
        console.print(
            f"[yellow]{source}: response is not a valid {fmt.upper()} "
            f"(first bytes {payload[:8]!r})[/yellow]"
        )
        return None
    return save_bytes(payload, output_dir, name)


def parse_gutenberg_search(page: str) -> list[dict]:
    """Parse a Gutenberg search page into rows. Pure, so the fixture can test it."""
    rows: list[dict] = []
    # Gutenberg encodes each result as <li class="booklink"> with a title and
    # author span. The search page has no language column, so rows start with
    # an optimistic "English" and gutenberg_book() confirms the real language
    # before a file is written.
    for block in re.split(r'<li class="booklink">', page)[1:]:
        match = re.search(r'href="/(ebooks/(\d+))"', block)
        if not match:
            continue
        title = re.search(r'<span class="title">(.*?)</span>', block, re.S)
        author = re.search(r'<span class="subtitle">(.*?)</span>', block, re.S)
        rows.append(
            {
                "source": "gutenberg",
                "id": match.group(2),
                "url": f"{GUTENBERG_URL}/{match.group(1)}",
                "title": html.unescape(re.sub(r"<[^>]+>", "", title.group(1))).strip()
                if title
                else "",
                "author": html.unescape(re.sub(r"<[^>]+>", "", author.group(1))).strip()
                if author
                else "",
                "language": "English",
                "format": "epub",
            }
        )
    return rows


def gutenberg_search(query: str, deadline: float) -> list[dict]:
    started = time.monotonic()
    url = f"{GUTENBERG_URL}/ebooks/search/?" + urllib.parse.urlencode(
        {"query": query}
    )
    try:
        page = fetch(url, timeout=max(5.0, min(30.0, remaining(deadline)))).decode(
            "utf-8", "replace"
        )
    except Exception as error:
        skip_note("Project Gutenberg", f"search failed ({type(error).__name__})", started)
        return []
    rows = parse_gutenberg_search(page)
    if not rows:
        skip_note("Project Gutenberg", "no result", started)
    return rows


def parse_gutenberg_book(page: str, book_id: str) -> tuple[str, str]:
    """Return `(epub_url, language)` from a Gutenberg book page."""
    language = ""
    meta = re.search(
        r'property="dcterms:language"[^>]*content="([^"]+)"', page
    )
    if meta:
        language = language_name(meta.group(1))
    else:
        row = re.search(r"<th>Language</th>\s*<td>(.*?)</td>", page, re.S)
        if row:
            language = html.unescape(re.sub(r"<[^>]+>", "", row.group(1))).strip()
    link = re.search(r'href="([^"]+\.epub3\.images)"', page)
    epub_url = (
        urllib.parse.urljoin(GUTENBERG_URL, html.unescape(link.group(1)))
        if link
        else f"{GUTENBERG_URL}/ebooks/{book_id}.epub3.images"
    )
    return epub_url, language


def gutenberg_book(book_id: str, deadline: float) -> tuple[str, str]:
    page = fetch(
        f"{GUTENBERG_URL}/ebooks/{book_id}",
        timeout=max(5.0, min(30.0, remaining(deadline))),
    ).decode("utf-8", "replace")
    return parse_gutenberg_book(page, book_id)


def gutenberg_download(
    query: str,
    fmt: str,
    output_dir: str,
    *,
    expected_title: Optional[str],
    expected_author: Optional[str],
    expected_isbn: Optional[str],
    expected_language: Optional[str],
) -> Optional[Path]:
    started = time.monotonic()
    deadline = started + SOURCE_BUDGET_SECONDS
    console.print("[dim]Project Gutenberg: searching…[/dim]")
    rows = gutenberg_search(query, deadline)
    if not rows:
        return None

    ranked = rank_candidates(
        rows,
        expected_title=expected_title,
        expected_author=expected_author,
        expected_isbn=expected_isbn,
        expected_language=expected_language,
        fmt=fmt,
    )
    if not ranked:
        console.print(f"[yellow]Project Gutenberg: no {fmt} rows[/yellow]")
        return None
    console.print(
        f"[dim]Project Gutenberg: best match {ranked[0][0]:.2f} · "
        f"{(ranked[0][1].get('title') or '?')[:80]} · "
        f"{ranked[0][1].get('author') or 'unknown'}[/dim]"
    )

    attempted = 0
    for score, row in ranked:
        if score < MATCH_THRESHOLD:
            break
        if attempted >= 5 or remaining(deadline) < 12:
            break
        attempted += 1
        try:
            epub_url, language = gutenberg_book(row["id"], deadline)
        except Exception as error:
            console.print(
                f"[yellow]Project Gutenberg: {row['id']} page failed "
                f"({type(error).__name__})[/yellow]"
            )
            continue
        confirmed = candidate_score(
            expected_title=expected_title,
            expected_author=expected_author,
            expected_isbn=expected_isbn,
            expected_language=expected_language,
            title=row.get("title") or "",
            author=row.get("author") or "",
            language=language,
            isbn_blob=row.get("title") or "",
        )
        if confirmed < MATCH_THRESHOLD:
            console.print(
                f"[dim]Project Gutenberg: skip {row['title'][:60]!r} "
                f"({language or 'unknown language'}, {confirmed:.2f})[/dim]"
            )
            continue
        try:
            payload = fetch(
                epub_url,
                timeout=max(10.0, min(90.0, remaining(deadline) - 5)),
            )
        except Exception as error:
            console.print(
                f"[yellow]Project Gutenberg: download failed "
                f"({type(error).__name__})[/yellow]"
            )
            continue
        saved = save_verified_payload(
            payload,
            output_dir,
            safe_name(expected_title or row["title"]) + ".epub",
            fmt,
            "Project Gutenberg",
        )
        if saved:
            return saved

    skip_note("Project Gutenberg", "no verified EPUB", started)
    return None


def parse_standardebooks_search(page: str) -> list[dict]:
    """Parse a Standard Ebooks search page into rows (pure, fixture-testable)."""
    rows: list[dict] = []
    # Each result is <li typeof="schema:Book" about="/ebooks/<author>/<title>">
    # with schema:name for the title and an author card for the name.
    for block in re.split(r'<li typeof="schema:Book"', page)[1:]:
        match = re.search(r'about="(/ebooks/[^"]+)"', block)
        if not match:
            continue
        title = re.search(r'property="schema:name">(.*?)</span>', block, re.S)
        author = re.search(
            r'class="author".*?property="schema:name">(.*?)</span>', block, re.S
        )
        rows.append(
            {
                "source": "standardebooks",
                "path": match.group(1),
                "url": f"{STANDARDEBOOKS_URL}{match.group(1)}",
                "title": html.unescape(re.sub(r"<[^>]+>", "", title.group(1))).strip()
                if title
                else "",
                "author": html.unescape(re.sub(r"<[^>]+>", "", author.group(1))).strip()
                if author
                else "",
                # Standard Ebooks only publishes English editions.
                "language": "English",
                "format": "epub",
            }
        )
    return rows


def standardebooks_search(query: str, deadline: float) -> list[dict]:
    started = time.monotonic()
    url = f"{STANDARDEBOOKS_URL}/ebooks?" + urllib.parse.urlencode({"query": query})
    try:
        page = fetch(url, timeout=max(5.0, min(30.0, remaining(deadline)))).decode(
            "utf-8", "replace"
        )
    except Exception as error:
        skip_note("Standard Ebooks", f"search failed ({type(error).__name__})", started)
        return []
    rows = parse_standardebooks_search(page)
    if not rows:
        skip_note("Standard Ebooks", "no result", started)
    return rows


def standardebooks_download_url(page: str, path: str) -> str:
    """Return a direct file URL, bypassing the "Your Download Has Started" page.

    Standard Ebooks answers the plain `/downloads/<slug>.epub` URL with an
    XHTML interstitial that meta-refreshes to the same path with
    `?source=download`; only that second URL returns `application/epub+zip`.
    Fetching the first URL anyway is what produced the HTML-payload rejection
    during the NF-032 probe.
    """
    links = re.findall(r'href="([^"]+\.epub)"', page)
    plain = [
        link
        for link in links
        if "_advanced" not in link and not link.endswith(".kepub.epub")
    ]
    if plain:
        url = urllib.parse.urljoin(STANDARDEBOOKS_URL, html.unescape(plain[0]))
    else:
        # Fallback: /ebooks/<author>/<title> → /downloads/<author>_<title>.epub
        slug = "_".join(part for part in path.strip("/").split("/")[1:])
        url = f"{STANDARDEBOOKS_URL}{path}/downloads/{slug}.epub"
    separator = "&" if "?" in url else "?"
    return f"{url}{separator}source=download"


def standardebooks_download(
    query: str,
    fmt: str,
    output_dir: str,
    *,
    expected_title: Optional[str],
    expected_author: Optional[str],
    expected_isbn: Optional[str],
    expected_language: Optional[str],
) -> Optional[Path]:
    started = time.monotonic()
    deadline = started + SOURCE_BUDGET_SECONDS
    console.print("[dim]Standard Ebooks: searching…[/dim]")
    rows = standardebooks_search(query, deadline)
    if not rows:
        return None

    matches = choose_candidates(
        rows,
        source="Standard Ebooks",
        expected_title=expected_title,
        expected_author=expected_author,
        expected_isbn=expected_isbn,
        expected_language=expected_language,
        fmt=fmt,
        limit=3,
    )
    if not matches:
        # choose_candidates stays quiet; run the single-candidate reporter once
        # so the near misses are visible.
        choose_candidate(
            rows,
            source="Standard Ebooks",
            expected_title=expected_title,
            expected_author=expected_author,
            expected_isbn=expected_isbn,
            expected_language=expected_language,
            fmt=fmt,
        )
        return None

    for chosen in matches:
        if remaining(deadline) < 12:
            break
        try:
            page = fetch(
                chosen["url"],
                timeout=max(5.0, min(30.0, remaining(deadline))),
            ).decode("utf-8", "replace")
        except Exception as error:
            console.print(
                f"[yellow]Standard Ebooks: {chosen['title'][:60]} page failed "
                f"({type(error).__name__})[/yellow]"
            )
            continue
        download_url = standardebooks_download_url(page, chosen["path"])
        try:
            payload = fetch(
                download_url,
                timeout=max(10.0, min(90.0, remaining(deadline) - 5)),
            )
        except Exception as error:
            console.print(
                f"[yellow]Standard Ebooks: download failed "
                f"({type(error).__name__})[/yellow]"
            )
            continue
        saved = save_verified_payload(
            payload,
            output_dir,
            safe_name(expected_title or chosen["title"]) + ".epub",
            fmt,
            "Standard Ebooks",
        )
        if saved:
            return saved

    skip_note("Standard Ebooks", "no verified EPUB", started)
    return None


# --------------------------------------------------------------------------- #
# Anna's Archive — Playwright with DDoS-Guard wait
# --------------------------------------------------------------------------- #


async def annas_download(
    query: str,
    fmt: str,
    language: Optional[str],
    output_dir: str,
    headless: bool,
    *,
    expected_title: Optional[str],
    expected_author: Optional[str],
    expected_isbn: Optional[str],
) -> Optional[Path]:
    from playwright.async_api import async_playwright

    started = time.monotonic()
    deadline = started + SOURCE_BUDGET_SECONDS
    console.print("[dim]Anna's Archive: searching…[/dim]")
    params = {"q": query, "ext": fmt}
    if language:
        params["lang"] = language
    search_url = f"{ANNAS_URL}/search?{urllib.parse.urlencode(params)}"

    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(
            headless=headless,
            args=["--disable-blink-features=AutomationControlled", "--no-sandbox"],
        )
        try:
            context = await browser.new_context(user_agent=USER_AGENT, accept_downloads=True)
            page = await context.new_page()
            await page.goto(
                search_url,
                wait_until="domcontentloaded",
                timeout=max(5000, int(remaining(deadline) * 1000)),
            )

            # DDoS-Guard holds the page for ~20 seconds, then replaces the document.
            # Both title() and evaluate() can land mid-navigation, so every probe is
            # best-effort and simply retried. The deadline keeps a blocked mirror
            # from burning the whole job budget.
            results = []
            search_tokens = query_tokens(query)
            while remaining(deadline) > 2.0:
                await page.wait_for_timeout(2500)
                try:
                    title = await page.title()
                    if "DDoS" in title or "moment" in title.lower():
                        continue
                    await page.wait_for_selector(
                        "a[href*='/md5/']",
                        timeout=min(8000, int(remaining(deadline) * 1000)),
                    )
                    results = await page.evaluate(
                        """(tokens) => {
                            const seen = new Set();
                            const rows = [];
                            for (const link of document.querySelectorAll("a[href*='/md5/']")) {
                                const md5 = (link.getAttribute('href') || '').match(/\\/md5\\/([a-f0-9]{32})/i)?.[1];
                                if (!md5 || seen.has(md5)) continue;
                                const box = link.closest('div') || link.parentElement;
                                const text = (box?.innerText || link.innerText || '').replace(/\\s+/g, ' ').trim();
                                const title = (link.innerText || link.getAttribute('title') || '').replace(/\\s+/g, ' ').trim();
                                if (!title) continue;
                                // The page also links unrelated books in sidebars and
                                // carousels. Keep only rows that mention a query token,
                                // otherwise every candidate scores ~0 on an empty title.
                                if (tokens.length && !tokens.some(t => text.toLowerCase().includes(t))) continue;
                                seen.add(md5);
                                rows.push({ md5, href: link.getAttribute('href'), title, text: text.slice(0, 400) });
                            }
                            return rows;
                        }""",
                        search_tokens,
                    )
                    if results:
                        break
                except Exception:
                    continue
            if not results:
                skip_note(
                    "Anna's Archive",
                    "no result (DDoS-Guard challenge or empty search)",
                    started,
                )
                return None

            rows = [
                {
                    "source": "annas",
                    "md5": row.get("md5") or "",
                    "title": row.get("title") or "",
                    "author": "",
                    "language": language or "",
                    "format": fmt,
                    "text": row.get("text") or "",
                    "href": row.get("href") or "",
                }
                for row in results
                if fmt.lower() in (row.get("text") or "").lower()
            ]
            if not rows:
                rows = [
                    {
                        "source": "annas",
                        "md5": row.get("md5") or "",
                        "title": row.get("title") or "",
                        "author": "",
                        "language": language or "",
                        "format": fmt,
                        "text": row.get("text") or "",
                        "href": row.get("href") or "",
                    }
                    for row in results
                ]
            chosen = choose_candidate(
                rows,
                source="Anna's Archive",
                expected_title=expected_title,
                expected_author=expected_author,
                expected_isbn=expected_isbn,
                expected_language=language,
                fmt=fmt,
            )
            if not chosen:
                return None
            console.print(f"[dim]Anna's Archive:[/dim] {(chosen.get('title') or query)[:90]}")

            book_url = urllib.parse.urljoin(ANNAS_URL, chosen["href"])
            await page.goto(
                book_url,
                wait_until="domcontentloaded",
                timeout=max(5000, int(remaining(deadline) * 1000)),
            )
            await page.wait_for_timeout(2500)

            links = await page.evaluate(
                """() => Array.from(document.querySelectorAll('a')).map(a => ({
                    href: a.href, text: (a.innerText || '').trim().slice(0, 80)
                })).filter(x => x.href && (x.href.includes('/slow_download/') || x.href.includes('/fast_download/')))"""
            )
            slow = [item for item in links if "/slow_download/" in item["href"]]
            if not slow:
                skip_note("Anna's Archive", "no slow download link", started)
                return None

            await page.goto(
                urllib.parse.urljoin(ANNAS_URL, slow[0]["href"]),
                wait_until="domcontentloaded",
                timeout=max(5000, int(remaining(deadline) * 1000)),
            )
            attempts = 0
            while remaining(deadline) > 6.0:
                attempts += 1
                await page.wait_for_timeout(min(4000, int(remaining(deadline) * 1000)))
                try:
                    button = await page.query_selector(
                        'a:has-text("Download now"), a:has-text("📚"), a[href*=".epub"]'
                    )
                    if not button:
                        continue
                    console.print("[dim]Anna's Archive: download ready[/dim]")
                    timeout_ms = int(min(45000, remaining(deadline) * 1000))
                    async with page.expect_download(timeout=timeout_ms) as download_info:
                        await button.click()
                    download = await download_info.value
                    name = safe_name(
                        expected_title or download.suggested_filename or f"{query}.{fmt}",
                        f"book.{fmt}",
                    )
                    if not name.lower().endswith(f".{fmt.lower()}"):
                        name += f".{fmt.lower()}"
                    target = Path(output_dir) / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    await download.save_as(str(target))
                    console.print(f"[green]DOWNLOADED:[/green] {target} ({target.stat().st_size / 1024 / 1024:.2f} MB)")
                    return target
                except Exception:
                    if attempts == 5:
                        # The slow page sometimes exposes the CDN URL directly instead.
                        direct = await page.evaluate(
                            """() => Array.from(document.querySelectorAll('a'))
                                .map(a => a.href)
                                .find(h => h.includes('.epub')) || ''"""
                        )
                        if direct:
                            downloaded = download_direct(direct, output_dir, safe_name(query) + f".{fmt}")
                            if downloaded:
                                return downloaded
                    console.print(
                        f"[dim]Anna's Archive: waiting "
                        f"({time.monotonic() - started:.0f}s elapsed)[/dim]"
                    )
            skip_note(
                "Anna's Archive",
                "download did not start before the budget",
                started,
            )
            return None
        finally:
            await browser.close()


# --------------------------------------------------------------------------- #
# OceanOfPDF — nodriver headed, Cloudflare clearance + POST chain
# --------------------------------------------------------------------------- #


async def ocean_download(
    query: str,
    fmt: str,
    output_dir: str,
    *,
    expected_title: Optional[str],
    expected_author: Optional[str],
    expected_isbn: Optional[str],
    expected_language: Optional[str],
) -> Optional[Path]:
    try:
        import nodriver as uc
        from nodriver import cdp
    except ImportError:
        console.print("[yellow]OceanOfPDF: nodriver is not installed — skipped[/yellow]")
        return None

    started = time.monotonic()
    deadline = started + SOURCE_BUDGET_SECONDS
    headful = os.environ.get("NOVELFORGE_OCEAN_HEADFUL") == "1"
    console.print(
        f"[dim]OceanOfPDF: opening {'visible (debug)' if headful else 'headless'} browser…[/dim]"
    )
    profile = Path.home() / ".cache" / "novelforge" / "ocean-profile"
    profile.mkdir(parents=True, exist_ok=True)
    # A cancelled/killed run can leave Chromium's singleton lock behind; the
    # next run then dies with "Failed to connect to browser" before Turnstile is
    # even attempted. Only one Ocean job runs at a time, so clearing them is safe.
    for stale in ("SingletonLock", "SingletonSocket", "SingletonCookie"):
        try:
            (profile / stale).unlink()
        except OSError:
            pass
    browser_args = ["--no-sandbox", "--disable-blink-features=AutomationControlled"]
    if headful:
        browser_args += ["--window-position=-2000,-2000", "--window-size=1280,900"]
    browser_kwargs = {
        "headless": not headful,
        "user_data_dir": str(profile),
        "browser_args": browser_args,
    }
    chromium = os.environ.get("NOVELFORGE_CHROMIUM")
    if chromium is None and sys.platform.startswith("linux"):
        # The Linux toolchain installs distro Chromium. Elsewhere let nodriver
        # discover the browser, or use the Playwright Chromium path that
        # NovelForge injects on Windows.
        chromium = "/usr/bin/chromium"
    if chromium:
        browser_kwargs["browser_executable_path"] = chromium
    browser = await uc.start(**browser_kwargs)
    try:
        page = await browser.get(f"{OCEAN_URL}/?s={urllib.parse.quote(query)}")

        # Turnstile replaces the document after ~50-60s. Poll for actual result
        # anchors instead of trusting the title, which is briefly empty during
        # navigation.
        links = []
        while remaining(deadline) > 2.0:
            await asyncio.sleep(min(3, remaining(deadline)))
            try:
                raw = await page.evaluate(
                    """(() => Array.from(document.querySelectorAll("a[href*='/authors/']"))
                        .map(a => a.href)
                        .filter((href, index, all) => all.indexOf(href) === index))()"""
                )
            except Exception:
                continue
            links = [item["value"] if isinstance(item, dict) else item for item in (raw or [])]
            if links:
                break
        if not links:
            skip_note(
                "OceanOfPDF",
                "no book result (Cloudflare/Turnstile challenge or empty search)",
                started,
            )
            return None

        tokens = query_tokens(query)

        def score(url: str) -> int:
            return sum(1 for token in tokens if token in url.lower())

        # Never trust the first search hit. Open a few candidates and compare
        # the page heading with the selected catalogue record.
        links.sort(key=score, reverse=True)
        book_url = ""
        for candidate in links[:6]:
            if remaining(deadline) <= 3.0:
                break
            try:
                await page.get(candidate)
                await asyncio.sleep(min(2, remaining(deadline)))
                heading = await page.evaluate(
                    "document.querySelector('h1')?.innerText || document.title"
                )
            except Exception:
                continue
            heading = clean_source_heading((heading or "").strip())
            if not expected_title:
                book_url = candidate
                break
            if title_similarity(expected_title, heading) >= MATCH_THRESHOLD:
                console.print(f"[dim]OceanOfPDF:[/dim] {heading[:90]}")
                book_url = candidate
                break
            console.print(
                f"[dim]OceanOfPDF: skip {heading[:60]!r} "
                f"(title {title_similarity(expected_title, heading):.2f})[/dim]"
            )
        if not book_url:
            skip_note(
                "OceanOfPDF",
                "no result matched the requested title",
                started,
            )
            return None
        form_json = ""
        while remaining(deadline) > 3.0:
            await asyncio.sleep(min(3, remaining(deadline)))
            try:
                title = await page.evaluate("document.title")
                if "moment" in title.lower() or "attention" in title.lower():
                    continue
                form_json = await page.evaluate(
                    """(() => {
                        const form = Array.from(document.querySelectorAll("form[action*='Fetching_Resource']"))
                            .find(f => (f.querySelector("input[name='filename']")?.value || '').toLowerCase().endsWith('.epub'))
                            || Array.from(document.querySelectorAll("form[action*='Fetching_Resource']"))[0];
                        if (!form) return '';
                        const data = new FormData(form);
                        return JSON.stringify({
                            action: form.action,
                            id: data.get('id') || '',
                            filename: data.get('filename') || ''
                        });
                    })()"""
                )
            except Exception:
                continue
            if form_json:
                break
        if not form_json:
            skip_note("OceanOfPDF", "no download form (Cloudflare hold)", started)
            return None
        console.print(f"[dim]OceanOfPDF:[/dim] {form_json[:120]}")
        form = __import__("json").loads(form_json)
        if fmt.lower() != "epub" or not form["filename"].lower().endswith(".epub"):
            skip_note("OceanOfPDF", f"no {fmt} edition on this page", started)
            return None

        cookies = await page.send(cdp.network.get_cookies())
        clearance = next((cookie for cookie in cookies if cookie.name == "cf_clearance"), None)
        user_agent = await page.evaluate("navigator.userAgent")
        cookie_header = f"cf_clearance={clearance.value}" if clearance else ""

        step_one = fetch(
            form["action"],
            data={"id": form["id"], "filename": form["filename"]},
            headers={
                "User-Agent": user_agent,
                "Cookie": cookie_header,
                "Referer": book_url,
                "Content-Type": "application/x-www-form-urlencoded",
            },
            timeout=max(10, int(remaining(deadline))),
        ).decode("utf-8", "replace")
        action = re.search(r'<form[^>]*action="([^"]+)"', step_one, re.I)
        if not action:
            skip_note("OceanOfPDF", "intermediate form missing", started)
            return None
        step_two = fetch(
            html.unescape(action.group(1)),
            data={"filename": form["filename"], "id": form["id"]},
            headers={
                "User-Agent": user_agent,
                "Cookie": cookie_header,
                "Referer": form["action"],
            },
            timeout=max(10, int(remaining(deadline))),
        ).decode("utf-8", "replace")
        direct = re.search(r'var DOWNLOAD_URL = "([^"]+)"', step_two)
        if not direct:
            skip_note("OceanOfPDF", "direct URL missing", started)
            return None
        url = direct.group(1).replace("\\u0026", "&").replace("\\/", "/")
        return download_direct(url, output_dir, safe_name(expected_title or query) + ".epub")
    finally:
        browser.stop()


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #


def resolve_sources(source: str) -> Iterable[str]:
    if source in ("auto", ""):
        return SOURCES
    return (source,)


@click.command()
@click.argument("query", required=False)
@click.option("--format", "-f", "fmt", default="epub", help="epub (default) or pdf")
@click.option("--lang", "-L", default=None, help="Language code / name passed to the source")
@click.option("--output", "-o", default=str(Path.home() / "Downloads" / "NovelForge"),
              help="Output directory")
@click.option("--headless/--headed", default=True, help="Headless for Anna's Archive")
@click.option("--limit", "-l", default=20, help="Max search rows reported by --search-json")
@click.option("--url", default=ANNAS_URL, help="Anna's Archive mirror")
@click.option("--auto/--no-auto", default=False, help="Take the best match without prompting")
@click.option("--title", default=None, help="Expected title for verification")
@click.option("--author", default=None, help="Expected author for verification")
@click.option("--isbn", default=None, help="Expected ISBN for verification")
@click.option("--search-json", is_flag=True, default=False,
              help="Print LibGen candidates as JSON and exit (no download)")
@click.option("--source", "-s", type=click.Choice(["auto", *ALL_SOURCES]), default="auto",
              help="Source order: auto, libgen, welib, annas, ocean, gutenberg, standardebooks")
@click.option("--budget", default=None, type=float,
              help="Per-source time budget in seconds (default 150, env NOVELFORGE_SOURCE_BUDGET)")
@click.option("--key", "-k", envvar="ANNAS_KEY", help="Anna's Archive member key (unused)")
def main(query, fmt, lang, output, headless, limit, url, auto, source, key, title, author,
         isbn, search_json, budget):
    """Download a book from LibGen, WeLib, Anna's Archive, OceanOfPDF,
    Project Gutenberg or Standard Ebooks."""
    del auto, key
    global SOURCE_BUDGET_SECONDS
    if budget is not None:
        SOURCE_BUDGET_SECONDS = max(20.0, float(budget))
    if not query:
        click.echo(
            "Usage: aget <title or author> "
            "[--source auto|libgen|welib|annas|ocean|gutenberg|standardebooks] "
            "[--title T] [--author A] [--isbn I]"
        )
        return

    fmt = (fmt or "epub").lower().strip(".")
    global ANNAS_URL
    ANNAS_URL = url.rstrip("/")

    if search_json:
        rows = libgen_rows(query, fmt)[: max(limit or 20, 1)]
        for row in rows:
            row.pop("base", None)
        json.dump(rows, sys.stdout, ensure_ascii=False, indent=2)
        sys.stdout.write("\n")
        return

    failures = []

    for name in resolve_sources(source):
        source_started = time.monotonic()
        try:
            if name == "libgen":
                result = libgen_download(
                    query,
                    fmt,
                    output,
                    expected_title=title,
                    expected_author=author,
                    expected_isbn=isbn,
                    expected_language=lang,
                )
            elif name == "welib":
                result = welib_download(
                    query,
                    output,
                    expected_title=title,
                    expected_author=author,
                    expected_isbn=isbn,
                    expected_language=lang,
                )
            elif name == "gutenberg":
                result = gutenberg_download(
                    query,
                    fmt,
                    output,
                    expected_title=title,
                    expected_author=author,
                    expected_isbn=isbn,
                    expected_language=lang,
                )
            elif name == "standardebooks":
                result = standardebooks_download(
                    query,
                    fmt,
                    output,
                    expected_title=title,
                    expected_author=author,
                    expected_isbn=isbn,
                    expected_language=lang,
                )
            elif name == "annas":
                result = asyncio.run(
                    annas_download(
                        query,
                        fmt,
                        lang,
                        output,
                        headless,
                        expected_title=title,
                        expected_author=author,
                        expected_isbn=isbn,
                    )
                )
            else:
                result = asyncio.run(
                    ocean_download(
                        query,
                        fmt,
                        output,
                        expected_title=title,
                        expected_author=author,
                        expected_isbn=isbn,
                        expected_language=lang,
                    )
                )
            if result:
                console.print(
                    f"[green]{name}: success in "
                    f"{time.monotonic() - source_started:.1f}s[/green]"
                )
                return
            failures.append(name)
            console.print(
                f"[dim]{name}: no download in "
                f"{time.monotonic() - source_started:.1f}s[/dim]"
            )
        except Exception as error:
            console.print(
                f"[yellow]{name}: {type(error).__name__}: {error} "
                f"after {time.monotonic() - source_started:.1f}s[/yellow]"
            )
            failures.append(name)

    console.print(f"[red]No download completed ({', '.join(failures)}).[/red]")
    sys.exit(1)


if __name__ == "__main__":
    main()
