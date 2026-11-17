#!/usr/bin/env python3
"""Measure book sources on the same titles, end to end (NF-030).

Every row is a real `aget` run with expected title/author, so a "success" means
the source produced an EPUB that cleared the match threshold. The output is a
JSON record plus a Markdown table that can be pasted into
`docs/SOURCE_PIPELINE_BACKLOG.md`.

    ~/.local/share/novelforge/tools/aget-venv/bin/python \
        scripts/measure_sources.py --budget 150

The default titles mix public-domain classics with catalogue titles. Nothing is
mocked: a blocked/challenged source is recorded as a failure with its reason.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import statistics
import subprocess
import sys
import time
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
AGET = ROOT / "scripts" / "aget.py"

TITLES = [
    ("Pride and Prejudice", "Jane Austen"),
    ("The Great Gatsby", "F. Scott Fitzgerald"),
    ("The Time Machine", "H. G. Wells"),
    ("Dune", "Frank Herbert"),
    ("Fourth Wing", "Rebecca Yarros"),
]
SOURCES = ["libgen", "welib", "annas", "ocean"]


def slug(value: str) -> str:
    return "".join(character if character.isalnum() else "-" for character in value.lower()).strip("-")


def is_epub(path: Path) -> bool:
    try:
        with path.open("rb") as handle:
            return handle.read(4) == b"PK\x03\x04"
    except OSError:
        return False


def summarise(output: str) -> str:
    """Keep the lines that explain the result, not the whole log.

    `no download` lines are printed for every source after the real reason, so
    they must not crowd out the deciding line (`… — skipped`, `no row cleared`,
    `DOWNLOADED`). Segments are capped per line so a long output path cannot
    dominate the table cell.
    """
    interesting = []
    for line in output.splitlines():
        clean = line.strip()
        if not clean:
            continue
        lowered = clean.lower()
        # Rich wraps console lines, so a wrapped `… — skipped` can arrive as a
        # fragment on its own physical line. Ignore those.
        if len(clean) < 15:
            continue
        if any(
            marker in lowered
            for marker in (
                "— skipped",
                "blocked (",
                "no row cleared",
                "downloaded:",
                "download failed",
                "out of time",
                "best match",
                "no epub",
                "not installed",
            )
        ):
            interesting.append(clean[:110])
    if not interesting:
        tail = [line.strip() for line in output.splitlines() if line.strip()]
        return (tail[-1][:110] if tail else "")
    return " | ".join(interesting[-2:])[:240]


def measure(
    title: str,
    author: str,
    source: str,
    *,
    python: str,
    budget: float,
    work_dir: Path,
) -> dict:
    out_dir = work_dir / source / slug(title)
    # A file left by an earlier attempt must never make a failed run look
    # successful, so every measurement starts from an empty directory.
    shutil.rmtree(out_dir, ignore_errors=True)
    out_dir.mkdir(parents=True, exist_ok=True)
    command = [
        python,
        str(AGET),
        title,
        "--title",
        title,
        "--author",
        author,
        "--source",
        source,
        "--output",
        str(out_dir),
        "--budget",
        str(budget),
    ]
    started = time.monotonic()
    log_path = out_dir / "aget.log"
    # A pipe would stay open if Playwright/Chromium grandchildren inherit it, so
    # the log goes straight to a file and the whole process group is killed on
    # timeout. COLUMNS keeps rich from wrapping a skip reason over two physical
    # lines, which would split the one line that explains the result.
    env = {**os.environ, "COLUMNS": "200"}
    with log_path.open("w") as log:
        process = subprocess.Popen(
            command,
            stdout=log,
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
            env=env,
        )
        try:
            exit_code = process.wait(timeout=budget + 120)
            timed_out = False
        except subprocess.TimeoutExpired:
            os.killpg(os.getpgid(process.pid), signal.SIGKILL)
            process.wait()
            exit_code = -1
            timed_out = True
    output = log_path.read_text()
    elapsed = time.monotonic() - started

    epubs = sorted(
        path for path in out_dir.rglob("*.epub") if path.is_file() and is_epub(path)
    )
    chosen = epubs[0] if epubs else None
    return {
        "title": title,
        "author": author,
        "source": source,
        "success": bool(chosen) and exit_code == 0,
        "exit": exit_code,
        "timedOut": timed_out,
        "seconds": round(elapsed, 1),
        "format": "epub" if chosen else "",
        "bytes": chosen.stat().st_size if chosen else 0,
        "file": chosen.name if chosen else "",
        "log": str(log_path),
        "note": "timed out" if timed_out else summarise(output),
    }


def short_paths(note: str) -> str:
    """Drop the absolute output path; the Size column already has the bytes."""
    parts = []
    for part in note.split(" | "):
        if part.startswith("DOWNLOADED"):
            parts.append("DOWNLOADED")
        else:
            parts.append(re.sub(r"(?:/[\w.\-]+){2,}", "…", part))
    return " | ".join(parts)


def reason(note: str) -> str:
    """The deciding reason without per-run timings, for de-duplication."""
    parts = note.split(" | ")
    chosen = parts[0]
    for marker in ("skipped", "no row cleared", "not installed"):
        matches = [part for part in parts if marker in part]
        if matches:
            chosen = matches[-1]
            break
    chosen = re.sub(r"\s+after\s+\d+(?:\.\d+)?s", "", chosen)
    return chosen.strip(" -—")


def markdown(results: list[dict], budget: float) -> str:
    lines = [
        f"Measured {date.today().isoformat()} with a per-source budget of "
        f"{budget:.0f}s. Each row is a real `aget` run with expected "
        "title/author; success means a verified `PK` EPUB was written.",
        "",
        "| Title | Source | Result | Seconds | Size | Note |",
        "| --- | --- | --- | ---: | ---: | --- |",
    ]
    for row in results:
        result = "success" if row["success"] else "fail"
        size = f"{row['bytes'] / 1024 / 1024:.2f} MB" if row["bytes"] else "—"
        note = short_paths(row["note"]).replace("|", "/")
        lines.append(
            f"| {row['title']} | {row['source']} | {result} | "
            f"{row['seconds']:.1f} | {size} | {note} |"
        )

    lines += ["", "| Source | Success | Median seconds | Notes |", "| --- | ---: | ---: | --- |"]
    for source in SOURCES:
        rows = [row for row in results if row["source"] == source]
        if not rows:
            continue
        wins = sum(1 for row in rows if row["success"])
        # Median over successful runs only: a 150 s budget timeout is a failure,
        # and including it would report a "typical" success that never happened.
        timings = [row["seconds"] for row in rows if row["success"]] or [
            row["seconds"] for row in rows
        ]
        median = statistics.median(timings)
        failures = sorted({reason(row["note"]) for row in rows if not row["success"]})
        lines.append(
            f"| {source} | {wins}/{len(rows)} | {median:.1f} | "
            f"{'; '.join(failures)[:160] or 'all measured rows succeeded'} |"
        )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--budget", type=float, default=150.0)
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--sources", default=",".join(SOURCES))
    parser.add_argument("--titles", default="", help="Optional 'Title|Author;Title|Author' override")
    parser.add_argument("--work-dir", default="/tmp/opencode/nf-030-measure")
    parser.add_argument("--out", default=str(ROOT / "docs" / "measurements"))
    parser.add_argument(
        "--merge",
        nargs="*",
        default=None,
        help="Merge existing result JSON files (later files win) and write a combined report",
    )
    args = parser.parse_args()

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = date.today().isoformat()

    if args.merge:
        merged: dict[tuple[str, str], dict] = {}
        for path in args.merge:
            payload = json.loads(Path(path).read_text())
            for row in payload.get("results", []):
                merged[(row["title"], row["source"])] = row
        order = {title: index for index, (title, _) in enumerate(TITLES)}
        source_order = {source: index for index, source in enumerate(SOURCES)}
        results = sorted(
            merged.values(),
            key=lambda row: (
                order.get(row["title"], 99),
                source_order.get(row["source"], 99),
            ),
        )
        json_path = out_dir / f"nf-030-{stamp}.json"
        md_path = out_dir / f"nf-030-{stamp}.md"
        json_path.write_text(
            json.dumps(
                {"date": stamp, "budgetSeconds": args.budget, "results": results},
                indent=2,
            )
            + "\n"
        )
        md_path.write_text(markdown(results, args.budget))
        print(md_path)
        return 0 if all(row["success"] for row in results) else 1

    titles = TITLES
    if args.titles:
        titles = [
            tuple(part.split("|", 1))  # type: ignore[misc]
            for part in args.titles.split(";")
            if part.strip()
        ]
    sources = [value.strip() for value in args.sources.split(",") if value.strip()]
    work_dir = Path(args.work_dir) / date.today().isoformat()

    results = []
    for title, author in titles:
        for source in sources:
            sys.stderr.write(f"measuring {source}: {title}…\n")
            row = measure(
                title,
                author,
                source,
                python=args.python,
                budget=args.budget,
                work_dir=work_dir,
            )
            results.append(row)
            sys.stderr.write(
                f"  {'ok' if row['success'] else 'fail'} in {row['seconds']:.1f}s "
                f"({row['note'][:90]})\n"
            )

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = date.today().isoformat()
    json_path = out_dir / f"nf-030-{stamp}.json"
    md_path = out_dir / f"nf-030-{stamp}.md"
    payload = {"date": stamp, "budgetSeconds": args.budget, "results": results}
    json_path.write_text(json.dumps(payload, indent=2) + "\n")
    md_path.write_text(markdown(results, args.budget))
    print(md_path)
    return 0 if all(row["success"] for row in results) else 1


if __name__ == "__main__":
    sys.exit(main())
