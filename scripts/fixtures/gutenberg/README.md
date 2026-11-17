# Project Gutenberg fixtures

Real pages saved on 2026-11-16 for the offline NF-032 tests. No network access
is used when these are parsed.

| File | Page | Why |
| --- | --- | --- |
| `pride-search.html` | `https://www.gutenberg.org/ebooks/search/?query=pride+and+prejudice` | search-card parsing: `<li class="booklink">`, `span.title`, `span.subtitle`, `/ebooks/<id>` |
| `pride-1342.html` | `https://www.gutenberg.org/ebooks/1342` | book-page parsing: `dcterms:language` and the `.epub3.images` link |
| `les-miserables-135.html` | `https://www.gutenberg.org/ebooks/135` | the English edition of a title that also exists in French; confirms the language code is read from metadata, not guessed from the title |

The search page has no language column. Rows therefore start as "English" and
`parse_gutenberg_book` confirms the real language before a file is written;
that two-step behaviour is what the tests pin down.

Refresh only by saving a new live page and updating the assertions together.
