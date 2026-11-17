#!/usr/bin/env python3
"""Normalize `lncrawl sources list -o json` into the app's source catalogue.

Usage:
    lncrawl sources list -o json | python3 scripts/build_source_catalogue.py

The output is deterministic (sorted by language, then URL) so a refresh only
changes when lncrawl's inventory changes. Only enabled sources are emitted:
lncrawl omits disabled/rejected entries unless `--with-rejected` is passed, and
the app deliberately does not advertise sources the crawler refuses to open.
"""

from __future__ import annotations

import json
import sys
from typing import Any


def normalize(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    catalogue = []
    for row in rows:
        url = str(row.get("url") or "").strip()
        if not url.startswith(("http://", "https://")):
            continue
        features = row.get("features") or {}
        catalogue.append(
            {
                "url": url,
                "language": str(row.get("language") or "").strip(),
                "search": bool(features.get("Search")),
                "manga": bool(features.get("Manga")),
                "mtl": bool(features.get("MTL")),
            }
        )
    catalogue.sort(key=lambda item: (item["language"], item["url"]))
    return catalogue


def main() -> int:
    try:
        rows = json.load(sys.stdin)
    except json.JSONDecodeError as error:
        print(f"invalid lncrawl JSON: {error}", file=sys.stderr)
        return 2
    if not isinstance(rows, list):
        print("expected a JSON array from `lncrawl sources list -o json`", file=sys.stderr)
        return 2
    json.dump(normalize(rows), sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
