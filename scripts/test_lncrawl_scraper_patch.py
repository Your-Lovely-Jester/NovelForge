"""Offline gate for the lncrawl-scraper patches (NF-033, NF-040)."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
FIXTURE = ROOT / "fixtures" / "novelfire" / "readnovel-loading.html"


def load_patcher():
    spec = importlib.util.spec_from_file_location(
        "patch_lncrawl_scraper", ROOT / "patch_lncrawl_scraper.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ScraperPatchTests(unittest.TestCase):
    def setUp(self) -> None:
        self.patcher = load_patcher()

    def test_source_rewrite_is_idempotent(self) -> None:
        original = "    return found.group(1)\n"
        patched, changed = self.patcher.apply_to_source(original)
        self.assertTrue(changed)
        self.assertIn('replace("\\\\/", "/")', patched)

        again, changed_again = self.patcher.apply_to_source(patched)
        self.assertFalse(changed_again)
        self.assertEqual(again, patched)

    def test_ambiguous_source_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            self.patcher.apply_to_source(
                "return found.group(1)\nreturn found.group(1)\n"
            )

    def test_saved_mirror_page_has_an_escaped_destination(self) -> None:
        destination = self.patcher.fixture_destination()
        self.assertIn("\\/\\/novelphoenix.com", destination)
        self.assertEqual(
            destination.replace("\\/", "/"),
            "https://novelphoenix.com/novel/reverend-insanity/chapter-7?noti=1",
        )

    def test_fixture_is_a_real_loading_stub(self) -> None:
        body = FIXTURE.read_text(encoding="utf-8")
        self.assertLess(len(body), 4_000)
        self.assertIn("Loading...", body)
        self.assertIn("window.location.href", body)

    def test_pacing_patch_is_idempotent(self) -> None:
        original = "class Pacer:\n" + self.patcher.PACING_ORIGINAL
        patched, changed = self.patcher.apply_pacing_to_source(original)
        self.assertTrue(changed)
        self.assertIn("if origin in self._interval:", patched)

        again, changed_again = self.patcher.apply_pacing_to_source(patched)
        self.assertFalse(changed_again)
        self.assertEqual(again, patched)

    def test_pacing_patch_rejects_a_missing_body(self) -> None:
        with self.assertRaises(ValueError):
            self.patcher.apply_pacing_to_source("class Pacer:\n    pass\n")

    def test_pacing_patch_keeps_one_store_assignment(self) -> None:
        patched, _ = self.patcher.apply_pacing_to_source(
            "class Pacer:\n" + self.patcher.PACING_ORIGINAL
        )
        self.assertEqual(patched.count("self._interval[origin] = min("), 1)
        self.assertLess(
            patched.index("if origin in self._interval:"),
            patched.index("self._interval[origin] = min("),
        )


if __name__ == "__main__":
    unittest.main()
