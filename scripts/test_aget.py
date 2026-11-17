"""Offline regression tests for book metadata scoring and selection (NF-031).

Run from the repository root:

    python3 -m unittest discover -s scripts -t scripts -p 'test_*.py'

Everything here is offline. The LibGen fixtures under `scripts/fixtures/libgen`
are real search pages saved on 2026-11-16, so `parse_libgen`, the score
thresholds and `choose_candidate` are exercised against the same HTML the app
sees, without a network call. A scoring tweak that regresses to first-row
selection, accepts an omnibus, or ignores a declared foreign language fails
this file and therefore `scripts/check.sh`.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import aget

# The CLI prints near-miss diagnostics to the console; unittest output stays
# readable if the module's console is silenced for the duration of the suite.
aget.console.quiet = True

FIXTURES = Path(__file__).resolve().parent / "fixtures" / "libgen"
LIBGEN_BASE = "https://libgen.li"


def load_rows(name: str, fmt: str = "epub") -> list[dict]:
    page = (FIXTURES / f"{name}.html").read_text(encoding="utf-8")
    return aget.parse_libgen(page, LIBGEN_BASE, fmt)


def score(
    title: str,
    author: str,
    *,
    expected_title: str,
    expected_author: str | None,
    language: str = "English",
    expected_language: str | None = None,
) -> float:
    return aget.candidate_score(
        expected_title=expected_title,
        expected_author=expected_author,
        expected_isbn=None,
        expected_language=expected_language,
        title=title,
        author=author,
        language=language,
        isbn_blob="",
    )


class LibGenFixtureParsing(unittest.TestCase):
    def test_fourth_wing_page_parses_english_rows(self) -> None:
        rows = load_rows("fourth-wing")
        self.assertGreaterEqual(len(rows), 8)
        exact = [
            row
            for row in rows
            if row["title"] == "Fourth Wing"
            and row["author"] == "Rebecca Yarros"
            and row["language"] == "English"
        ]
        self.assertTrue(exact, "expected at least one exact English Fourth Wing row")
        for row in exact:
            self.assertEqual(row["format"], "epub")
            self.assertRegex(row["md5"], r"^[a-f0-9]{32}$")
            self.assertEqual(row["source"], "libgen")

    def test_decoy_page_parses_expected_titles(self) -> None:
        rows = load_rows("dune")
        titles = {row["title"] for row in rows}
        self.assertIn("Dune: The Gateway Collection", titles)
        self.assertIn("Dune: Tales of Dune Expanded Edition: Brian Herbert & Kevin J Anderson", titles)
        self.assertTrue(
            any(row["language"] == "Spanish" and row["title"] == "Dune 2a ed" for row in rows)
        )

    def test_unrelated_page_yields_no_rows(self) -> None:
        self.assertEqual(aget.parse_libgen("<html><body>no table</body></html>", LIBGEN_BASE, "epub"), [])


class SelectionOnRealPages(unittest.TestCase):
    def test_choose_candidate_picks_the_exact_book(self) -> None:
        chosen = aget.choose_candidate(
            load_rows("fourth-wing"),
            source="LibGen",
            expected_title="Fourth Wing",
            expected_author="Rebecca Yarros",
            expected_isbn=None,
            expected_language=None,
            fmt="epub",
        )
        self.assertIsNotNone(chosen)
        self.assertEqual(chosen["title"], "Fourth Wing")
        self.assertEqual(chosen["author"], "Rebecca Yarros")
        self.assertEqual(chosen["language"], "English")
        self.assertAlmostEqual(chosen["score"], 1.0, places=2)

    def test_decoy_only_page_fails_closed_and_writes_nothing(self) -> None:
        with tempfile.TemporaryDirectory() as output_dir:
            chosen = aget.choose_candidate(
                load_rows("dune"),
                source="LibGen",
                expected_title="Dune",
                expected_author="Frank Herbert",
                expected_isbn=None,
                expected_language=None,
                fmt="epub",
            )
            self.assertIsNone(chosen)
            self.assertEqual(list(Path(output_dir).iterdir()), [])

    def test_choose_candidates_is_sorted_bounded_and_thresholded(self) -> None:
        matches = aget.choose_candidates(
            load_rows("fourth-wing"),
            source="LibGen",
            expected_title="Fourth Wing",
            expected_author="Rebecca Yarros",
            expected_isbn=None,
            expected_language=None,
            fmt="epub",
            limit=3,
        )
        self.assertTrue(matches)
        self.assertLessEqual(len(matches), 3)
        scores = [row["score"] for row in matches]
        self.assertEqual(scores, sorted(scores, reverse=True))
        for row in matches:
            self.assertGreaterEqual(row["score"], aget.MATCH_THRESHOLD)
            self.assertEqual(row["language"], "English")

    def test_limit_is_respected(self) -> None:
        matches = aget.choose_candidates(
            load_rows("fourth-wing"),
            source="LibGen",
            expected_title="Fourth Wing",
            expected_author="Rebecca Yarros",
            expected_isbn=None,
            expected_language=None,
            fmt="epub",
            limit=1,
        )
        self.assertEqual(len(matches), 1)


class ScoringGuards(unittest.TestCase):
    def assertAccepted(self, value: float, label: str) -> None:
        self.assertGreaterEqual(value, aget.MATCH_THRESHOLD, label)

    def assertRejected(self, value: float, label: str) -> None:
        self.assertEqual(value, 0.0, label)

    def test_exact_title_is_accepted(self) -> None:
        self.assertAccepted(
            score("Dune", "Herbert, Frank", expected_title="Dune", expected_author="Frank Herbert"),
            "exact Dune",
        )

    def test_title_floor_rejects_wrong_volume(self) -> None:
        self.assertRejected(
            score("Dune Messiah", "Herbert, Frank", expected_title="Dune", expected_author="Frank Herbert"),
            "wrong volume",
        )

    def test_omnibus_and_collection_are_rejected(self) -> None:
        cases = (
            ("Dune: The Gateway Collection", "Herbert, Frank", "Dune", "Frank Herbert"),
            (
                "Frank Herbert's Dune Saga Collection: Books 1 - 6",
                "Frank Herbert,",
                "Dune",
                "Frank Herbert",
            ),
            (
                "The Complete Works of Jane Austen (In One Volume)",
                "Jane Austen",
                "Pride and Prejudice",
                "Jane Austen",
            ),
        )
        for title, author, expected_title, expected_author in cases:
            self.assertRejected(
                score(
                    title,
                    author,
                    expected_title=expected_title,
                    expected_author=expected_author,
                ),
                title,
            )

    def test_punctuation_suffix_needs_author_confirmation(self) -> None:
        self.assertRejected(
            score(
                "Dune: Tales of Dune Expanded Edition: Brian Herbert & Kevin J Anderson",
                "Frank Herbert; Anderson, Brian Herbert and Kevin J.",
                expected_title="Dune",
                expected_author="Frank Herbert",
            ),
            "derivative work trading on the Dune prefix",
        )
        self.assertAccepted(
            score(
                "The Great Gatsby: The Authentic Edition from Fitzgerald's Original Publisher",
                "F. Scott Fitzgerald",
                expected_title="The Great Gatsby",
                expected_author="F. Scott Fitzgerald",
            ),
            "legitimate subtitled edition",
        )
        self.assertAccepted(
            score(
                "Fourth Wing (The Empyrean, #1)",
                "Rebecca Yarros",
                expected_title="Fourth Wing",
                expected_author="Rebecca Yarros",
            ),
            "legitimate series suffix",
        )

    def test_declared_foreign_language_is_rejected_by_default(self) -> None:
        self.assertRejected(
            score(
                "Dune 2a ed",
                "Herbert, Frank;Santos, Domingo",
                expected_title="Dune",
                expected_author="Frank Herbert",
                language="Spanish",
            ),
            "Spanish edition",
        )
        self.assertRejected(
            score(
                "Fourth wing",
                "Rebecca Yarros",
                expected_title="Fourth Wing",
                expected_author="Rebecca Yarros",
                language="French",
            ),
            "French edition with an otherwise exact normalized title",
        )

    def test_explicit_language_still_selects_that_language(self) -> None:
        self.assertAccepted(
            score(
                "Fourth wing",
                "Rebecca Yarros",
                expected_title="Fourth Wing",
                expected_author="Rebecca Yarros",
                language="French",
                expected_language="French",
            ),
            "explicit French request",
        )
        self.assertRejected(
            score(
                "Fourth wing",
                "Rebecca Yarros",
                expected_title="Fourth Wing",
                expected_author="Rebecca Yarros",
                language="French",
                expected_language="German",
            ),
            "language mismatch",
        )

    def test_english_variants_are_accepted(self) -> None:
        # LibGen sometimes writes "English (US)" or "English "; the rejection
        # must not turn a formatting variant into a foreign language.
        for label in ("English (US)", "English ", "en", "en-GB"):
            self.assertAccepted(
                score(
                    "Dune",
                    "Herbert, Frank",
                    expected_title="Dune",
                    expected_author="Frank Herbert",
                    language=label,
                ),
                label,
            )

    def test_coauthor_row_does_not_score_full_author_match(self) -> None:
        similarity = aget.author_similarity(
            "Frank Herbert", "Frank Herbert; Brian Herbert & Kevin J Anderson"
        )
        self.assertLess(similarity, aget.PREFIX_AUTHOR_FLOOR)
        self.assertRejected(
            score(
                "Sisterhood of Dune Brian Herbert & Kevin J Anderson",
                "Frank Herbert; Brian Herbert & Kevin J Anderson",
                expected_title="Dune",
                expected_author="Frank Herbert",
            ),
            "sequel by co-authors",
        )

    def test_fuzzy_punctuation_title_is_rejected(self) -> None:
        self.assertRejected(
            score(
                "Dune. La casa Atreides 4a ed",
                "Anderson, Kevin J.;Murillo, Ed",
                expected_title="Dune",
                expected_author="Frank Herbert",
                language="Spanish",
            ),
            "translated spin-off",
        )


class TitleOnlySources(unittest.TestCase):
    """Anna's Archive rows carry a title but no author metadata."""

    def test_exact_title_renormalises_above_threshold(self) -> None:
        value = score(
            "Dune",
            "",
            expected_title="Dune",
            expected_author="Frank Herbert",
            language="",
        )
        self.assertGreaterEqual(value, aget.MATCH_THRESHOLD)

    def test_fuzzy_title_without_author_fails_closed(self) -> None:
        value = score(
            "Dune Messiah",
            "",
            expected_title="Dune",
            expected_author="Frank Herbert",
            language="",
        )
        self.assertLess(value, aget.MATCH_THRESHOLD)


class SourceHeadingCleanup(unittest.TestCase):
    def test_ocean_heading_is_cleaned_before_matching(self) -> None:
        self.assertEqual(
            aget.clean_source_heading("[PDF] [EPUB] Fourth Wing Download"),
            "Fourth Wing",
        )
        self.assertEqual(
            aget.clean_source_heading("[PDF] [EPUB] The Time Machine: An Invention Download"),
            "The Time Machine: An Invention",
        )

    def test_cleaned_heading_matches_the_expected_title(self) -> None:
        heading = aget.clean_source_heading("[PDF] [EPUB] Fourth Wing Download")
        self.assertGreaterEqual(
            aget.title_similarity("Fourth Wing", heading), aget.MATCH_THRESHOLD
        )


class ThresholdContract(unittest.TestCase):
    def test_published_thresholds(self) -> None:
        # These values are documented in SOURCE_PIPELINE_BACKLOG.md NF-030.
        # Changing one requires updating the docs and the fixture expectations.
        self.assertEqual(aget.MATCH_THRESHOLD, 0.62)
        self.assertEqual(aget.MIN_TITLE_SCORE, 0.55)
        self.assertEqual(aget.PREFIX_AUTHOR_FLOOR, 0.8)


class PublicDomainSources(unittest.TestCase):
    """Project Gutenberg and Standard Ebooks are opt-in lawful sources."""

    def fixture(self, relative: str) -> str:
        return (FIXTURES.parent / relative).read_text(encoding="utf-8")

    def test_auto_order_excludes_public_domain_sources(self) -> None:
        self.assertEqual(tuple(aget.resolve_sources("auto")), aget.SOURCES)
        self.assertEqual(
            tuple(aget.resolve_sources("")), aget.SOURCES
        )
        for name in aget.PUBLIC_DOMAIN_SOURCES:
            self.assertNotIn(name, aget.SOURCES)
            self.assertIn(name, aget.ALL_SOURCES)
            self.assertEqual(tuple(aget.resolve_sources(name)), (name,))

    def test_gutenberg_search_fixture_parses_and_selects(self) -> None:
        rows = aget.parse_gutenberg_search(self.fixture("gutenberg/pride-search.html"))
        self.assertGreaterEqual(len(rows), 5)
        first = rows[0]
        self.assertEqual(first["id"], "1342")
        self.assertEqual(first["title"], "Pride and Prejudice")
        self.assertEqual(first["author"], "Jane Austen")
        self.assertEqual(first["source"], "gutenberg")
        chosen = aget.choose_candidate(
            rows,
            source="Project Gutenberg",
            expected_title="Pride and Prejudice",
            expected_author="Jane Austen",
            expected_isbn=None,
            expected_language=None,
            fmt="epub",
        )
        self.assertIsNotNone(chosen)
        self.assertEqual(chosen["id"], "1342")

    def test_gutenberg_book_fixture_gives_language_and_epub_url(self) -> None:
        url, language = aget.parse_gutenberg_book(
            self.fixture("gutenberg/pride-1342.html"), "1342"
        )
        self.assertEqual(language, "English")
        self.assertEqual(url, "https://www.gutenberg.org/ebooks/1342.epub3.images")

    def test_gutenberg_language_codes_map_to_names(self) -> None:
        self.assertEqual(aget.language_name("en"), "English")
        self.assertEqual(aget.language_name("en-GB"), "English")
        self.assertEqual(aget.language_name("fr"), "French")
        self.assertEqual(aget.language_name("de"), "German")
        self.assertEqual(aget.language_name("xx"), "xx")

    def test_gutenberg_rejects_a_non_english_row(self) -> None:
        row = {
            "title": "Les Misérables",
            "author": "Victor Hugo",
            "language": "French",
        }
        self.assertEqual(
            aget.candidate_score(
                expected_title="Les Misérables",
                expected_author="Victor Hugo",
                expected_isbn=None,
                expected_language=None,
                title=row["title"],
                author=row["author"],
                language=row["language"],
                isbn_blob="",
            ),
            0.0,
        )

    def test_standardebooks_search_fixture_parses_and_selects(self) -> None:
        rows = aget.parse_standardebooks_search(
            self.fixture("standardebooks/pride-search.html")
        )
        self.assertGreaterEqual(len(rows), 1)
        self.assertEqual(rows[0]["path"], "/ebooks/jane-austen/pride-and-prejudice")
        self.assertEqual(rows[0]["title"], "Pride and Prejudice")
        self.assertEqual(rows[0]["author"], "Jane Austen")
        chosen = aget.choose_candidate(
            rows,
            source="Standard Ebooks",
            expected_title="Pride and Prejudice",
            expected_author="Jane Austen",
            expected_isbn=None,
            expected_language=None,
            fmt="epub",
        )
        self.assertIsNotNone(chosen)
        self.assertEqual(chosen["path"], rows[0]["path"])

    def test_standardebooks_url_bypasses_the_interstitial(self) -> None:
        book = self.fixture("standardebooks/pride-book.html")
        url = aget.standardebooks_download_url(
            book, "/ebooks/jane-austen/pride-and-prejudice"
        )
        self.assertEqual(
            url,
            "https://standardebooks.org/ebooks/jane-austen/pride-and-prejudice"
            "/downloads/jane-austen_pride-and-prejudice.epub?source=download",
        )
        self.assertNotIn("_advanced", url)
        self.assertNotIn(".kepub", url)

    def test_standardebooks_prefers_the_plain_epub(self) -> None:
        page = (
            '<a href="/x_advanced.epub">'
            '<a href="/x.kepub.epub">'
            '<a href="/x.epub">'
        )
        url = aget.standardebooks_download_url(page, "/ebooks/a/b")
        self.assertEqual(url, "https://standardebooks.org/x.epub?source=download")

    def test_standardebooks_falls_back_to_a_constructed_url(self) -> None:
        url = aget.standardebooks_download_url("<html></html>", "/ebooks/a/b")
        self.assertEqual(
            url, "https://standardebooks.org/ebooks/a/b/downloads/a_b.epub?source=download"
        )

    def test_html_payload_is_rejected_without_writing_a_file(self) -> None:
        self.assertFalse(aget.payload_matches_format(b"<!DOCTYPE html>", "epub"))
        self.assertTrue(aget.payload_matches_format(b"PK\x03\x04rest", "epub"))
        self.assertTrue(aget.payload_matches_format(b"%PDF-1.7", "pdf"))
        self.assertFalse(aget.payload_matches_format(b"PK\x03\x04", "pdf"))
        with tempfile.TemporaryDirectory() as output_dir:
            saved = aget.save_verified_payload(
                b"<!DOCTYPE html>" * 1000, output_dir, "x.epub", "epub", "Test"
            )
            self.assertIsNone(saved)
            self.assertEqual(list(Path(output_dir).iterdir()), [])



if __name__ == "__main__":
    unittest.main()
