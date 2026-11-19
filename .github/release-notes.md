NovelForge 0.1.5 — the first public build.

A private desktop download studio for **webnovels**, **manga** and **e-books**.
Three proven engines (`lncrawl`, `gallery-dl`, `aget`) run as real local
processes behind a dark glass UI, with live output, job controls and container
verification before anything reaches your library.

**Downloads**

- `novelforge-windows.zip` — unsigned NSIS installer and the engine setup scripts.
- `novelforge-linux.zip` — AppImage, `.deb` and the engine setup scripts.

Both archives contain `LICENSE`, `SHA256SUMS.txt` and the `scripts/` folder used
by the install block in the [README](https://github.com/Your-Lovely-Jester/NovelForge#install).
Check a download with `sha256sum -c SHA256SUMS.txt` on Linux or `Get-FileHash`
in PowerShell.

**Highlights**

- Webnovel crawls through `lncrawl`, with chapter ranges, resume, update checks
  and EPUB verification.
- Manga through `gallery-dl`: one CBZ per chapter, extractor metadata, and the
  series poster as the library cover.
- Books through `aget`, plus local EPUB import, rebuild and repair.
- Live terminal dock, transfer queue, disk-usage panel and library search.

**Notes**

- Builds are unsigned; Windows SmartScreen will warn on first launch.
- Windows runtime acceptance is still tracked in NF-057. These installers are
  produced by CI but have not yet been exercised on a stock WebView2 machine.

NovelForge is free software under **GPL-3.0-or-later**. The complete
corresponding source for this release is the `v0.1.5` tag of this repository.
