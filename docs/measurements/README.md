# Book source measurements

`scripts/measure_sources.py` runs the real `aget` pipeline for every
`(title, source)` pair and writes one JSON + Markdown pair per day:

```bash
~/.local/share/novelforge/tools/aget-venv/bin/python \
  scripts/measure_sources.py --budget 120
```

- **Success** means the run exited `0` and wrote a file whose first four bytes
  are `PK\x03\x04` (an EPUB container). Nothing is mocked.
- **Seconds** is the full source attempt (search + candidate scoring + file
  transfer), not just the HTTP request.
- **Note** is the deciding log line (`skipped`, `no row cleared`, `DOWNLOADED`,
  `best match`), so a blocked source is distinguishable from a missing book.
- **Median seconds** is computed over *successful* rows only. A 150 s budget
  timeout is a failure; including it would advertise a typical success that
  never happened.
- Each source is bounded by `--budget` (default 150 s). Playwright/Chromium
  grandchildren are killed with the process group on timeout.
- `--work-dir` defaults to `/tmp/opencode/nf-030-measure`; point it at a
  persistent path (`~/.cache/novelforge/nf-030-final`) if the machine may
  reboot mid-run.
- `--merge a.json b.json …` re-renders the report from saved results (later
  files win), so a single corrected row can replace a bad one without rerunning
  all twenty.

The generated files are committed as evidence for NF-030. Re-run them when a
source parser, mirror or scoring threshold changes, and update the NF-030 table
from the new Markdown.
