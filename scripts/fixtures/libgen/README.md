# LibGen search fixtures

These two files are **real search pages** saved on 2026-11-16, not hand-written
rows:

| File | Query | EPUB rows |
| --- | --- | ---: |
| `fourth-wing.html` | `Fourth Wing Rebecca Yarros` | 11 |
| `dune.html` | `Dune Frank Herbert` | 13 |

They are used by `scripts/test_aget.py` (NF-031) so `parse_libgen`,
`candidate_score`, `choose_candidate` and `choose_candidates` are tested against
the HTML the app actually sees, with no network access.

- `fourth-wing.html` contains four exact English editions plus foreign-language
  editions and a translated re-title. It proves the selector picks the English
  book and does not pick a French/Indonesian/Portuguese copy.
- `dune.html` contains only decoys (`Dune: The Gateway Collection`, the Brian
  Herbert/Anderson sequels, Spanish/Dutch editions, `Dune: Tales of Dune
  Expanded Edition`). It proves a page with no acceptable row fails closed and
  writes no file.

Do not edit these files to make a test pass. If a scoring rule changes the
expected outcome, change the assertion and the NF record in
`docs/SOURCE_PIPELINE_BACKLOG.md`. Refresh a fixture only by saving a new live
page and updating the counts and assertions together.
