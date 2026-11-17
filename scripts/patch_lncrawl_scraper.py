#!/usr/bin/env python3
"""Patch lncrawl-scraper 1.7.0 where its behaviour is wrong for NovelForge.

Two patches, both applied to the installed package the interpreter owns:

1. **Escaped JavaScript redirects** (`scraper.diagnosis`). NovelFire answers a
   chapter request with a 302 to its mirror network. The mirror
   (`readnovel.site`) replies with a small 200 "Loading..." page whose only
   payload is:

       window.location.href = "https:\\/\\/novelphoenix.com\\/novel\\/...\\/chapter-7?noti=1"

   `js_redirect` extracts the quoted string verbatim, so `urljoin()` treats the
   backslashes as path characters and builds a 404. Decoding the JSON-style
   `\\/` escape before returning the destination lets the hop land on the real
   mirror page.

2. **A learned interval that never recovers** (`scraper.pacing`). The session
   calls `Pacer.learn(origin, profile.interval)` before *every* request, and
   `learn()` unconditionally overwrites the in-memory target. `eased()` only
   narrows after ten consecutive successes, so the narrowing is erased by the
   next request and the persisted profile moves just ~1% per ten successes.
   A host that throttled once therefore keeps its widened interval for every
   later run: `novelfire.net` sat at 9.8 s/request on 2026-11-16 despite 401
   recorded successes (see docs/SOURCE_PIPELINE_BACKLOG.md, NF-040). The patch
   makes `learn()` adopt a profile only for an origin this pacer has not seen
   yet; an already-narrowed in-memory value is fresher and wins.

Usage:
    patch_lncrawl_scraper.py [--verify]

The interpreter running this script owns the installed scraper package. Run it
with the lncrawl virtualenv's Python after `pip install --editable
lightnovel-crawler`. `--verify` exercises both patches: `js_redirect` against
the saved `readnovel.site` page, and a `Pacer` ratchet simulation.
"""

from __future__ import annotations

import importlib.util
import re
import sys
from pathlib import Path

ORIGINAL = "return found.group(1)"
PATCHED = 'return found.group(1).replace("\\\\/", "/")'

PACING_ORIGINAL = '''    def learn(self, origin: str, interval: float) -> None:
        """Adopt a mean interval carried over from a previous run."""
        if interval <= 0:
            return
        with self._lock:
            self._interval[origin] = min(interval, self.policy.max_interval)
'''

PACING_PATCHED = '''    def learn(self, origin: str, interval: float) -> None:
        """Adopt a mean interval carried over from a previous run."""
        if interval <= 0:
            return
        with self._lock:
            # `Session` calls this before every request. Adopting the persisted
            # profile again here undoes every narrowing `eased()` has made, so a
            # widened interval only decays about 1% per ten successes and a
            # host that throttled once stays slow for every later run. The
            # in-memory value is this run's own, fresher measurement; only an
            # origin this pacer has not seen yet takes the profile.
            if origin in self._interval:
                return
            self._interval[origin] = min(interval, self.policy.max_interval)
'''

FIXTURE = (
    Path(__file__).resolve().parent
    / "fixtures"
    / "novelfire"
    / "readnovel-loading.html"
)
EXPECTED = "https://novelphoenix.com/novel/reverend-insanity/chapter-7?noti=1"

_DESTINATION = re.compile(
    r"""window\.location(?:\.replace|\.assign|\.href\s*=|\s*=)\s*\(?\s*['"]([^'"\s]+)['"]""",
    re.IGNORECASE,
)


def apply_to_source(source: str) -> tuple[str, bool]:
    """Return (source, changed). Idempotent and guarded against stray matches."""
    if PATCHED in source:
        return source, False
    if source.count(ORIGINAL) != 1:
        raise ValueError(
            f"expected exactly one {ORIGINAL!r}, found {source.count(ORIGINAL)}"
        )
    return source.replace(ORIGINAL, PATCHED, 1), True


def apply_pacing_to_source(source: str) -> tuple[str, bool]:
    """Return (source, changed) for the `Pacer.learn` ratchet fix."""
    if PACING_PATCHED in source:
        return source, False
    if source.count(PACING_ORIGINAL) != 1:
        raise ValueError(
            "expected exactly one Pacer.learn body, found "
            f"{source.count(PACING_ORIGINAL)}"
        )
    return source.replace(PACING_ORIGINAL, PACING_PATCHED, 1), True


def fixture_destination() -> str:
    return _DESTINATION.search(FIXTURE.read_text(encoding="utf-8")).group(1)


def verify() -> None:
    from scraper.diagnosis import js_redirect

    body = FIXTURE.read_text(encoding="utf-8")
    actual = js_redirect(body)
    if actual != EXPECTED:
        raise SystemExit(
            "patch_lncrawl_scraper: js_redirect returned "
            f"{actual!r}, expected {EXPECTED!r}"
        )
    print("patch_lncrawl_scraper: js_redirect decodes escaped destinations")


def verify_pacing() -> None:
    """Prove a narrowed interval survives the stale profile arriving again."""
    from scraper.pacing import Pacer, PacingPolicy

    pacer = Pacer(PacingPolicy(interval=0.5, floor=0.0), seed=7)
    pacer.learn("novelfire.net", 9.8)
    for _ in range(10):
        pacer.eased("novelfire.net")
    narrowed = pacer.interval_for("novelfire.net")
    if not narrowed < 9.8:
        raise SystemExit(
            "patch_lncrawl_scraper: Pacer.eased did not narrow a widened "
            f"interval (still {narrowed:.3f}s)"
        )

    # `Session` re-learns the persisted profile before every request. Before the
    # patch this line put 9.8 back and erased the recovery step.
    pacer.learn("novelfire.net", 9.8)
    kept = pacer.interval_for("novelfire.net")
    if kept != narrowed:
        raise SystemExit(
            "patch_lncrawl_scraper: Pacer.learn restored the stale interval "
            f"({kept:.3f}s) over the fresher {narrowed:.3f}s"
        )

    fresh = Pacer(PacingPolicy(interval=0.5, floor=0.0), seed=7)
    fresh.learn("new.example", 3.0)
    if fresh.interval_for("new.example") != 3.0:
        raise SystemExit(
            "patch_lncrawl_scraper: Pacer.learn stopped adopting an interval "
            "for an origin it had not seen"
        )
    print(
        "patch_lncrawl_scraper: Pacer.learn keeps the narrowed interval "
        f"({narrowed:.3f}s) and still adopts unseen origins"
    )


def _patch_source(path: Path, applier, label: str, verify_patched) -> bool:
    """Apply *applier* to *path*. False means the process should fail."""
    source = path.read_text(encoding="utf-8")
    try:
        patched, changed = applier(source)
    except ValueError as error:
        # A different scraper build may already behave correctly. Trust the
        # behaviour over the source shape, but only when the check passes.
        try:
            verify_patched()
        except SystemExit:
            print(f"patch_lncrawl_scraper: {path}: {error}", file=sys.stderr)
            return False
        print(
            f"patch_lncrawl_scraper: installed scraper already {label}; "
            f"no source patch needed ({path})"
        )
        return True

    if not changed:
        print(f"patch_lncrawl_scraper: already applied to {path}")
        return True

    path.write_text(patched, encoding="utf-8")
    print(f"patch_lncrawl_scraper: applied {label} to {path}")
    return True


def _origin(module: str) -> Path | None:
    spec = importlib.util.find_spec(module)
    if spec is None or spec.origin is None:
        return None
    return Path(spec.origin)


def main(argv: list[str]) -> int:
    if "--verify" in argv:
        verify()
        verify_pacing()
        return 0

    diagnosis = _origin("scraper.diagnosis")
    pacing = _origin("scraper.pacing")
    if diagnosis is None or pacing is None:
        print("patch_lncrawl_scraper: scraper is not installed", file=sys.stderr)
        return 1

    if not _patch_source(
        diagnosis,
        apply_to_source,
        "handles escaped redirects",
        verify,
    ):
        return 1
    if not _patch_source(
        pacing,
        apply_pacing_to_source,
        "recovers a narrowed pacing interval",
        verify_pacing,
    ):
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
