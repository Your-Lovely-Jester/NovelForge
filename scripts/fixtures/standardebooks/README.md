# Standard Ebooks fixtures

Real pages saved on 2026-11-16 for the offline NF-032 tests. No network access
is used when these are parsed.

| File | Page | Why |
| --- | --- | --- |
| `pride-search.html` | `https://standardebooks.org/ebooks?query=pride+and+prejudice` | search-card parsing: `<li typeof="schema:Book" about="/ebooks/<author>/<title>">`, `schema:name`, author card |
| `pride-book.html` | `https://standardebooks.org/ebooks/jane-austen/pride-and-prejudice` | the real download links (`compatible`, `.kepub`, `_advanced`) used to pick the plain EPUB |
| `pride-download-interstitial.html` | the plain `/downloads/<slug>.epub` URL | proves the plain URL is an XHTML "Your Download Has Started" page; the file is only served with `?source=download` |

The interstitial is the reason `standardebooks_download_url` appends
`?source=download`. Fetching the plain link returns HTML and must be rejected
without writing a file.

Refresh only by saving a new live page and updating the assertions together.
