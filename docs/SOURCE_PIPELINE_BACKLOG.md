# NovelForge — Source Expansion & Download Pipeline Backlog

**Last updated:** 2026-11-17  
**Status:** Active implementation  
**Owner:** NovelForge desktop app  
**Purpose:** Preserve the full defect list, source-coverage context, parser design, and throughput strategy across sessions. Update this file after every fix.

---

## How to use this file

- Every work item has a stable ID (`NF-001`, `NF-002`, …).
- Each item has a status:
  - `[ ] OPEN`
  - `[~] IN PROGRESS`
  - `[x] DONE`
  - `[!] BLOCKED`
- When an item is fixed:
  1. Change its status to `[x] DONE`.
  2. Add the date and a one-line evidence note under **Resolution**.
  3. Add an entry to the **Change log** at the bottom.
  4. Keep the original defect description intact so future sessions understand why the code exists.
- Do not delete completed items. This file is the historical record.
- Do not claim a source is "supported" unless it has passed the acceptance criteria for that source.

---

## 1. Current architecture snapshot

NovelForge is a Tauri desktop application:

- **Frontend:** React + TypeScript, Vite.
- **Backend:** Rust, Tauri commands.
- **Crawler:** external Python `lncrawl` installation, currently at:
  - `~/.local/share/novelforge/tools/lncrawl-venv`
  - source checkout `~/.local/share/novelforge/tools/lightnovel-crawler`
    (pinned `v4.14.0` + `scripts/patches/`)
  - User config: `~/.lncrawl/config.json`
- **Manga:** external `gallery-dl` (active; NF-022 replaced archived `kotatsu-dl`).
- **Books:** external `aget` with sources `auto`, `libgen`, `welib`, `annas`,
  `ocean` (`libgen` is the verified first choice; Ocean is explicit-only).
- **Downloads:** `lncrawl crawl <query> -f epub --chapters <spec> --noin`.
- **Structured export:** `src-tauri/src/export.rs` converts the lncrawl EPUB into:
  - `cover.*`
  - `metadata.json` with full chapter index
  - `index.json` compact chapter list
  - `chapters.html` searchable human-readable index
  - `chapters/*.xhtml`
  - one bundled EPUB
- **Verification:** `src-tauri/src/verify.rs` plus chapter-aware checks in `export.rs`.
- **Library:** `src-tauri/src/commands.rs` scans the download root and reads `metadata.json`.
- **Top-level webnovel index:** `src-tauri/src/metadata.rs`.
- **Webnovel search/resolver:** `src-tauri/src/resolver.rs`.
- **QA harness:** debug-only `src-tauri/src/qa.rs`, controlled by environment variables.

### Important files

| File | Responsibility |
|---|---|
| `src-tauri/src/process.rs` | Job orchestration, lncrawl invocation, retries, progress, output import |
| `src-tauri/src/metadata.rs` | Metadata extraction, cover cache, full chapter index fetch |
| `src-tauri/src/export.rs` | Audit, structured export, EPUB verification, library metadata reader |
| `src-tauri/src/chapter_clean.rs` | Chapter XHTML cleanup (duplicate titles, nav/ads, per-source chrome) |
| `src-tauri/src/verify.rs` | Container verification |
| `src-tauri/src/commands.rs` | Tauri commands, library scan, search command |
| `src-tauri/src/resolver.rs` | Title/synonym resolution, source search, manga source list |
| `src-tauri/src/model.rs` | Shared types including completeness flags |
| `src-tauri/src/qa.rs` | Debug-only end-to-end harness |
| `src/components/novels/NovelSearchPanel.tsx` | Search UI, metadata hydration, chapter-index preview |
| `src/pages/HomePage.tsx` | Shelf, source/genre filters, real library data |
| `src/pages/LibraryPage.tsx` | Structured novel cards + raw archive file table |
| `src/components/rail/*` | Progress, queue and library rail cards |

---

## 2. Verified current state (evidence)

These facts were verified live during development on 2026-11-14.

### 2.1 Live NovelFire run

- Book: **Reverend Insanity**
- Source: `https://novelfire.net/book/reverend-insanity`
- Selected chapters: 1–3.
- Full index: **2,334 chapters**.
- TOC: 24 pages × 100 chapters.
- Output:
  - 3 real XHTML chapters with non-empty text.
  - real cover (`cover.jpg`, 40,466 bytes).
  - `metadata.json` (about 559 KB because it carries the full 2,334-entry index).
  - one EPUB with 3 chapter entries.
- Run time: about **41.3 seconds** for 3 chapters, dominated by the 24-page TOC walk.
- `indexComplete=true`, `verified.ok=true`, `complete=true`.
- A 6-way parallel TOC fetch triggered HTTP 429 and produced only 634/2,334 chapters.
- Sequential TOC fetches with ~900 ms pacing and retries produced all 2,334 chapters.

### 2.2 Retry/failure E2E test

Using a local fixture server and fake lncrawl:

- First run produced chapters 1–2 out of 1–3.
- Job failed with `Incomplete output — empty chapter(s): 3`.
- Partial folder remained on disk with `complete=false` in `metadata.json`.
- With retry data fixed, the retry produced 3/3 non-empty chapters and completed.
- Home filters incomplete downloads out; Library still shows them for inspection.

### 2.3 Tests

- `cargo test`: **23 passed**.
- `npm run build`: **passed**.

### 2.4 Installed lncrawl source inventory

Current installed build (`lncrawl sources list`):

| Metric | Count |
|---|---:|
| Total entries including disabled/rejected | 541 |
| Enabled sources | **339** |
| Disabled sources | **202** |
| Enabled searchable sources | **49** |
| Enabled manga sources | **21** |
| Enabled searchable non-manga sources | **43** |
| Enabled Korean-language (`ko`) sources | **0** |

Enabled languages:

`ar, en, es, fr, id, ja, multi, pt, ru, tr, vi, zh`

There is **no raw Korean-language crawler** in this installation. Korean-origin novels can appear as English translations on English-language aggregators, but source catalogues change for copyright reasons.

### 2.5 Source search coverage

The default resolver search targets the NovelForge registry’s searchable
sources:

1. `https://novelfire.net/`
2. `https://novelphoenix.net/`

The studio can instead search one explicit searchable catalogue host or every
searchable non-manga source (`all`, 43 hosts). Direct-URL-only sources remain
reachable by pasting a link. See `search_novels_from` and `sources::search_sources`
in `src-tauri/src/resolver.rs` and `src-tauri/src/sources.rs`.

---

## 3. Prioritized defect and work list

### Summary table

| ID | Priority | Status | Short description |
|---|---|---|---|
| NF-001 | P0 | `[x] DONE` | Webnovel `jobs` and `throttle` controls are ignored |
| NF-002 | P0 | `[x] DONE` | Generic TOC parsing can report `complete=true` on a truncated index |
| NF-003 | P0 | `[x] DONE` | Source search only covers NovelFire + NovelPhoenix |
| NF-004 | P1 | `[x] DONE` | Full index is fetched twice, with no cache/reuse |
| NF-005 | P1 | `[x] DONE` | Bytes, speed and ETA measure the wrong location |
| NF-006 | P1 | `[x] DONE` | Duplicate output folders for the same novel |
| NF-007 | P1 | `[x] DONE` | No delete/retry/update actions for structured downloads |
| NF-008 | P1 | `[x] DONE` | No parser abstraction for expanding to more sources |
| NF-009 | P1 | `[x] DONE` | Korean/copyright availability is not represented in the UI or data |
| NF-010 | P1 | `[x] DONE` | Library has old/unstructured folders that never migrate |
| NF-011 | P2 | `[x] DONE` | No human-readable full chapter index file |
| NF-012 | P2 | `[x] DONE` | Chapter text can contain duplicate titles/site chrome/ads |
| NF-013 | P2 | `[x] DONE` | Home “All Sites” means “sources in shelf”, not all sources |
| NF-014 | P2 | `[x] DONE` | “Premium Edition” label implies a non-existent paid tier |
| NF-015 | P2 | `[x] DONE` | Test matrix is too small for a multi-source crawler |
| NF-016 | P2 | `[x] DONE` | No disk quota / cleanup policy for partial or old downloads |
| NF-017 | P0 | `[x] DONE` | Anti-bot efficiency and adaptive concurrency strategy |
| NF-018 | P2 | `[x] DONE` | Resolver relies on English title databases and exact source search |
| NF-019 | P1 | `[x] DONE` | NovelFull chapter adapter is stale (AJAX catalogue) |
| NF-020 | P1 | `[x] DONE` | Divine Dao Library adapter is stale (site redesign) |
| NF-021 | P1 | `[x] DONE` | NovelPing has no lncrawl source adapter |
| NF-022 | P1 | `[x] DONE` | Manga engine `kotatsu-dl` is archived; evaluate a maintained replacement |
| NF-023 | P2 | `[x] DONE` | NovelFull metadata keeps the site suffix and loses the author |
| NF-024 | P2 | `[ ] OPEN` | No free public binary release or donation channel |
| NF-025 | P2 | `[x] DONE` | Frontend interaction coverage stops at the Home shelf |
| NF-026 | P1 | `[x] DONE` | Author search misses novels that the site lists on its author page |
| NF-027 | P1 | `[x] DONE` | Search/resolve latency (crawler startup, candidate verification) |
| NF-028 | P0 | `[x] DONE` | Book downloads take the first search hit with no title/author verification |
| NF-029 | P1 | `[x] DONE` | Series automation: one-action queue, `Author/Series/NN - Title/` layout, skip-complete and per-book not-found reporting |
| NF-030 | P1 | `[x] DONE` | Source policy measured 2026-11-16 (5 titles × 4 sources); per-source budgets, fast-fail skips, omnibus/language/prefix guards; Ocean explicit-only headless. User-approved order `libgen → welib → annas → ocean` |
| NF-031 | P2 | `[x] DONE` | `scripts/test_aget.py` (31 offline tests) + real LibGen/Gutenberg/Standard Ebooks page fixtures; wired into `check.sh`; found and fixed a foreign-language selection bug |
| NF-032 | P2 | `[x] DONE` | Project Gutenberg + Standard Ebooks as **opt-in** lawful sources; verified EPUB end to end, HTML/missing responses rejected; `auto` order unchanged |
| NF-033 | P1 | `[x] DONE` | NovelFire mirror hop served a JSON-escaped JS redirect; lncrawl-scraper 1.7.0 did not decode `\/\/`, so ~25% of chapters 404ed. Patched + fixture; live 20/20 (was 15/20) and 7–8 2/2 |
| NF-034 | P2 | `[x] DONE` | Licensed MangaDex chapters are reported as licensing with the publisher host instead of `exit 4`; Guardrails + QA matrix updated |
| NF-035 | P2 | `[x] DONE` | Silent crawls are killed at 15 min (5 min once chapters flow; paused time excluded) and one retry is allowed: a stall resumes `--resume`, a zero-output transient restarts, validation failures never retry. Live: good run 2/2 in 47 s; stall+resume 2 invocations; fatal stub 1 invocation |
| NF-036 | P1 | `[x] DONE` | NovelFire chapters cost 3 HTTP hops (7.6 s/ch); ranking now prefers the one-hop NovelPhoenix host — 20/20 in 56 s (was 342 s), index 42 s (was 194 s) |
| NF-037 | P3 | `[x] DONE` | Full 2,334-chapter single-hop run: 2,334/2,334 in 1,113.8 s (~2.1 ch/s), 0×429/503/1015, 33.7 MB EPUB verified, total 19m 12.6s |
| NF-038 | P2 | `[x] DONE` | Full-length export compiled a regex per title comparison (5.4 M times). Compiled once now; full run exports in 16.0 s (was >37 min stalled) |
| NF-039 | P2 | `[x] DONE` | Chapters stream from lncrawl's per-chapter `.zst` store during the crawl; live 40-chapter run showed 5→40 files while downloading, export 0.4 s, 0 provisional leftovers, XHTML well-formed |
| NF-040 | P1 | `[x] DONE` | Learned pacing ratchet never recovered: `Pacer.learn()` re-applied the persisted interval before every request, so NovelFire stayed at 9.8 s/ch (≈6 ch/min). Patched; 500-request simulation 5.93 s → 0.50 s |
| NF-041 | P1 | `[x] DONE` | Measured crawl pace is persisted in `metadata.json` and shown on the Library card; updates inherit the folder's own pace (measured 94.38 ch/min → `expected 2 s`) and a `Faster source` action offers only a resolver-verified, measured-cheaper host as a new folder. Live: NovelFire rank 5 → NovelPhoenix rank 0; NovelPhoenix → “already fastest” |
| NF-042 | P3 | `[x] DONE` | Error-shaped stdout lines now feed `ErrorTail`, traceback headers/INFO rows are skipped, and a zero-output exit 0 reports `Source error · <final exception>`; live bad URL now says `No crawler found for the domain` (2.4 s, 1 launch, no retry) |
| NF-043 | P1 | `[x] DONE` | TOC rows left as `Chapter N` now get real names: borrow from a verified donor by visible chapter number (never overwriting source names) and recover the rest from the chapter page heading. RI 379/379, ORV 65/65, LOTM unchanged; bundled EPUB nav/NCX/headings retitled too |
| NF-044 | P1 | `[x] DONE` | Visible numbers are translated through the index row order before `--chapters` reaches lncrawl (ORV: visible 2-3 → serials 3-4); the audit also compares each EPUB heading with the selected number, so a shifted spec fails instead of exporting the neighbour. Live ORV 2-3 and 275-277 exact, RI 1-2 unchanged, ORV `111` fails before any folder |
| NF-045 | P0 | `[x] DONE` | `settings.json`, `metadata.json`, `index.json`, `chapters.html`, the job journal, the index cache and the retitled EPUB were written in place; a crash could truncate them. All use `fsutil::write_atomic` (temp + `rename`, `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` on Windows), and each JSON gains `schemaVersion` |
| NF-046 | P0 | `[x] DONE` | Closing the window killed running crawls with no prompt and no record. `CloseRequested` / `ExitRequested` now prevent exit while jobs are live, emit `app://quit-requested`, and the UI asks before `confirm_quit` stops the process group and writes an interrupted journal |
| NF-047 | P0 | `[x] DONE` | Jobs were memory-only, so Downloads was empty after a restart. `jobs.json` is written on every status change, insert, clear and shutdown; queued/running/paused/verifying rows restore as `Interrupted`, and the Downloads row offers Resume through the normal verified retry path |
| NF-048 | P1 | `[x] DONE` | No persistent log or issue report. Every log line is appended to `novelforge.log` (5 MiB rotation to `.log.1`, UTC timestamps) and About → **Copy diagnostics** emits version/platform/tool paths/settings/active jobs/last 200 lines with the home directory shortened to `~` |
| NF-049 | P1 | `[x] DONE` | Any render error left a white window. `src/components/ErrorBoundary.tsx` keeps a usable screen with the message, Reload and Copy error |
| NF-050 | P1 | `[x] DONE` | Two launches could race `settings.json` and the job journal. `tauri-plugin-single-instance` is registered and raises/focuses the existing window |
| NF-051 | P0 | `[x] DONE` | `signal_group()` was Unix-only, so Windows pause/resume/cancel silently lied. Process control is now `terminate_process`/`force_kill_process` (`SIGTERM`/`SIGKILL` to the group on Unix, `taskkill /PID /T [/F]` on Windows); pause/resume return an explicit unsupported error off Unix instead of flipping the status |
| NF-052 | P1 | `[x] DONE` | `install-tools.sh` pip-installed `gallery-dl`/`click`/`rich`/`playwright`/`nodriver` unpinned and offered no component choice. Versions are pinned (`gallery-dl 1.32.20`, `click 8.5.1`, `nodriver 0.50.99`, `playwright 1.65.0`, `rich 15.0.0`), `webnovel`/`manga`/`books`/`all` selection is supported, and native Windows stops with an explicit message |
| NF-053 | P2 | `[x] DONE` | `cargo fmt --check` failed and clippy reported 102 warnings with neither in CI. Both are clean and now gate `scripts/check.sh` and `.github/workflows/ci.yml` (`clippy -- -D warnings`) |
| NF-054 | P2 | `[x] DONE` | ZIP asset copy trusted `entry.name()`. `safe_entry_path` accepts only normal relative components, rejecting absolute paths, drive prefixes and `..` |
| NF-055 | P2 | `[x] DONE` | No schema version in persisted JSON. `model::SCHEMA_VERSION` is stamped into `metadata.json`, `index.json`, `settings.json` and `jobs.json` on write; readers ignore unknown keys |
| NF-056 | P2 | `[x] DONE` | A corrupt bundled `lncrawl-sources.json` hit `panic!` under `panic = "abort"`. It now logs to stderr and returns an empty catalogue so the app still opens |
| NF-057 | P1 | `[ ] OPEN` | Windows runtime validation is still unproven. **Wine cannot test a Tauri build** (WebView2 is unavailable). Packaging is now covered without a local VM: `.github/workflows/windows-build.yml` runs fmt/clippy (including the `#[cfg(windows)]` branches), tests, then `tauri build --bundles nsis` and uploads the installer + `SHA256SUMS.txt`. The native toolchain bootstrap now exists: `scripts/install-tools.ps1` installs lncrawl (editable + all eight patches), gallery-dl, aget and Playwright Chromium under `%LOCALAPPDATA%\novelforge` with `.cmd` shims in `%LOCALAPPDATA%\novelforge\bin`; no admin, no WSL, no PATH edit. The app discovers those shims via `tools::installed_bin`, and `process.rs` injects `NOVELFORGE_CHROMIUM` for Books. Remaining work is the acceptance list on a stock Windows 10/11 machine; a modified LTSC/Atlas image is only a best-effort smoke test. See `docs/RELEASE_READINESS.md` §4 |
| NF-058 | P2 | `[ ] OPEN` | Android is not a port: the app spawns `lncrawl`/`gallery-dl`/`aget` Python processes, which Android forbids. Options (rewrite, companion server, Termux-style bundle) are a spike, not a release task; see `docs/RELEASE_READINESS.md` |
| NF-059 | P1 | `[ ] OPEN` | Binary-only release work: code signing, SHA-256 checksums, update channel, installer pinning, and GPL-3.0 compliance for the GPL-derived `scripts/patches/*` that are distributed inside the build inputs |
| NF-060 | P2 | `[x] DONE` | The private tree still carries real saved-page fixtures because the offline tests need them, but nothing personal or copyrighted can reach an artifact: `scripts/check-personal-data.sh` (run by `check.sh`) fails on `/home/<user>/` paths in product inputs, on `bundle.resources` in `tauri.conf.json`, on `include_bytes!`/`include_str!` outside `resources/` before `#[cfg(test)]`, and on known fixture markers in product code; if a release binary exists it also greps it for fixture prose. The three personal paths in `docs/SOURCE_PIPELINE_BACKLOG.md` and the measurement JSON were rewritten to `~`, `scripts/__pycache__` was deleted and ignored, and `.gitattributes` export-ignores fixtures, internal docs, measurements and build outputs. No QA logs, EPUBs or downloaded libraries exist in the tree |
| NF-061 | P3 | `[x] DONE` | Dead-code and duplication audit with the chosen toolset. `knip` (wired into `scripts/check.sh`, CI and `npm run knip`) found one unused component (`HeroArt.tsx`), one unused devDependency (`@testing-library/user-event`), six unused exports (`BYTES_PER_GB`, `formatEta`, `formatPercent`, `statusMeta`, `engineMeta`, `truncateMiddle`) and eleven internally-used-only types; all were removed or unexported. Eight unused custom CSS classes, the hero-art mask and four unreferenced `cover-*.jpg` assets were deleted. `cargo-machete` reports no unused Rust dependencies, `ruff` F-checks and `vulture --min-confidence 80` are clean, and `tsc --noUnusedLocals/Parameters` is clean. Rust dead-code: `sources::for_url` removed, `SourceProfile::validate`/`sources::all` moved under `#[cfg(test)]`, `ManifestItem.media_type` dropped, unused `core:window:allow-is-maximized` capability removed. Duplicates now share one implementation: `timeutil::civil_from_days`, `cache::key_hash`, `paths::is_hidden_path`, `lib/utils.ts::hostFromUrl`, and `src/test/factories.ts::makeNovel`. Seven unreferenced `scripts/fixtures/novelphoenix/*.html` pages were deleted. Gate stays green at **201** Rust lib + **2** integration + **36** frontend + **38** Python |


---

## NF-001 — Webnovel `jobs` and `throttle` controls are ignored

**Status:** `[x] DONE`  
**Priority:** P0  
**Area:** `src-tauri/src/process.rs`, `build_args`, `NovelSearchPanel.tsx`, `CrawlOptions`

### Evidence

`build_args()` passes `-j` and `--throttle` only in the `Engine::Manga` branch. For `Engine::Webnovel`, the generated command is effectively:

```text
lncrawl -ll crawl <query> -f epub --chapters <spec> --noin
```

There is no jobs argument, no rate-limit argument and no throttle argument. The UI sends `options.jobs` and `options.throttle`, but they have no effect for webnovels.

lncrawl's current user config has:

```json
{
  "crawler": {
    "runner_concurrency": 5,
    "max_attempts": 5,
    "runner_cooldown": 1
  }
}
```

So a webnovel job actually runs at lncrawl's default concurrency, not the number shown in the UI.

> **Correction added after the fix:** `crawler.runner_concurrency` is the number of concurrent
> crawl **jobs** in lncrawl's scheduler, not the number of chapter requests within one crawl.
> It was never a valid target for the webnovel “Parallel jobs” slider. The CLI's real
> per-run pacing control is `--rate-limit`.

### Why it matters

- User-facing settings are misleading.
- “Speed up” or “be gentle” has no effect.
- We cannot tune per-source concurrency while keeping the UI honest.
- It makes performance testing invalid because the app setting is not the real setting.

### Fix options

1. **Minimum viable fix**
   - Map the UI throttle toggle to lncrawl's `--rate-limit` argument.
   - Hide the webnovel jobs selector until it is wired, or label it “lncrawl default”.
2. **Preferred fix**
   - Add a per-source concurrency policy.
   - For lncrawl-based jobs, pass a supported rate limit; if lncrawl cannot accept a concurrency flag for `crawl`, write a temporary config override or use its environment/config API.
   - Do not silently accept an option the backend ignores.

### Acceptance criteria

- [x] With throttle enabled, the invoked lncrawl command/workspace visibly uses a rate limit.
- [x] With jobs changed in the UI, the effective request concurrency changes, or the control is removed for webnovels.
- [x] The generated command is logged in debug mode with effective options.
- [x] A test asserts that webnovel options reach the command/config, not only the manga path.

### Resolution

**Fixed:** 2026-11-14.

- Webnovel jobs now append `--rate-limit 2` when the throttle toggle is on.
- Throttle off appends no rate argument, leaving lncrawl’s source-default pacing.
- The same rate-limit arguments are applied to completeness retries, not only the first run.
- The launch log now states the effective transport, and the full command line (including
  `--rate-limit 2`) is logged before execution.
- The webnovel “Parallel jobs” slider was removed. `lncrawl crawl` has no per-run
  chapter-concurrency flag; `crawler.runner_concurrency` is the number of concurrent crawl
  **jobs**, not chapter requests. Keeping the slider would have remained misleading.
- Manga keeps its real `-j` / `--throttle` controls.
- Webnovel UI now says “Caps source requests at 2/sec.” or “Uses the source’s default
  pacing.” depending on the toggle.

**Verification:**

- `process::artifact_tests::webnovel_throttle_maps_to_lncrawl_rate_limit` — asserts
  `--rate-limit 2` is present and that `jobs: 16` never becomes `-j 16`.
- `process::artifact_tests::webnovel_throttle_off_keeps_source_default_pacing` — asserts
  the flag is absent when throttle is off.
- `cargo test`: **25 passed**.
- `npm run build`: **passed**.
- Live CLI acceptance: `lncrawl -ll crawl --noin --rate-limit 2` parsed the flag and
  proceeded to URL validation instead of rejecting the option.
- Live QA crawl using the new webnovel path produced a valid EPUB artifact; the existing
  structured-export and completeness tests remain the regression gate for post-processing.

---

## NF-002 — Generic TOC parsing can report `complete=true` on a truncated index

**Status:** `[x] DONE (2026-11-14)**  
**Priority:** P0  
**Area:** `src-tauri/src/metadata.rs`, `fetch_generic_index`, `parse_generic_metadata`, `process.rs` index completeness check

### Evidence

- `fetch_chapter_index()` only special-cases:
  - `novelfire.*`
  - `novelphoenix.*`
  - everything else falls through to `fetch_generic_index()`.
- `fetch_generic_index()`:
  - fetches **one** HTML page,
  - selects `a[href*="chapter"]`,
  - dedupes the links,
  - returns `complete: true` unconditionally.
- `parse_generic_metadata()` does not derive a chapter count for generic sources. It leaves `chapter_count` as `None` unless another path fills it.
- The job-level check compares index total against `metadata.chapter_count`. If that value is `None`, a short index passes.

### Why it matters

- A paginated table of contents can return only the first 20–100 chapters and still be treated as a complete 2,000-chapter index.
- The app's strongest guarantee — “downloads fail if the full index is missing or short” — is only true for the two specialized sources.
- Expanding to more sources without fixing this would produce confidently wrong output.

### Fix plan

1. Add a **TOC discovery** step:
   - locate the real table-of-contents link on the book page,
   - follow pagination links or page query parameters.
2. Add **multiple completeness signals**:
   - discovered page count,
   - advertised total chapter count from metadata/JSON-LD/text (“Total chapters: N”, “Latest chapter N”),
   - last chapter number/URL on the TOC,
   - first/last chapter numbers after parsing,
   - expected count from the job selection,
   - lncrawl/EPUB chapter count as a cross-check.
3. If the source exposes pagination but the walker stops early, return an error, not `complete=true`.
4. If no total is available, mark the index as `complete=false` unless the parser can prove it saw the terminal page of the TOC.
5. Preserve a diagnostic detail string: pages read, links found, advertised count, reason for uncertainty.
6. Never compare only against the selected chapter range. Completeness is about the whole source index.

### Acceptance criteria

- [x] A generic paginated TOC fixture returns every chapter, not only page 1.
- [x] A deliberately truncated generic TOC is rejected and the job fails.
- [x] A generic source with no discoverable total does not silently claim completeness.
- [x] Tests cover: single-page TOC, `?page=N` pagination, path pagination, “next” link pagination, and no pagination but advertised count mismatch.
- [x] `metadata.json` includes a diagnostic reason when `indexComplete=false`.

### Resolution

Implemented 2026-11-14 in `src-tauri/src/metadata.rs` and `src-tauri/src/process.rs`.

**Generic index walker**

- `fetch_generic_index()` no longer reads one page and assumes success.
- TOC entry URLs drop only pagination query keys (`page`, `paged`) while preserving unrelated query parameters.
- Chapter links are read from scoped containers first (largest container wins):
  - `#idData`, `#chapter-archive`, `[data-chapter-archive-grid]`, `[data-chapter-item]`
  - `.chapter-group__list` (Divine Dao / Fictioneer), `ul.chapter-list`, `#list-chapter`, `#chapter-list`, `.chapters`, `.chapter`, `[data-chapter-list]`, `table#chapters`, `.eplister`
  - `<template>` contents are expanded before parsing.
  - A filtered `a[href]` fallback is used only when no scoped container matches.
- Pagination discovery supports:
  - explicit `data-total-page`, `#total-page`, `data-total-pages`
  - query pagination `?page=N` / `?paged=N`
  - path pagination `/page/N`
  - `<link rel="next">` / `a[rel="next"]` chains
  - sequential page walks with the existing retry/backoff policy and a 250-page safety cap.
- AJAX archives are queried when the HTML list is short or empty:
  - `[data-list-url]` + `[data-novel-id]` + `[data-total-page]` (NovelFull)
  - inline `ajaxChapterArchiveUrl` / `ajaxChapterListUrl` / `chapterArchiveUrl`
  - `ajaxUrl + "/chapter-archive"` when the page has an archive grid (NovelPing)
  - JSON wrappers with an `html` field are unwrapped; same-origin endpoints only.
- JSON-LD `ItemList` is traversed through `@graph`; `numberOfItems` and the actual item count are both used, and item URLs are merged into the index when HTML does not list them all.

**Completeness proof**

An index is `complete=true` only when at least one of these holds:

1. an advertised total exists and the listed count meets or exceeds it;
2. a JSON-LD `ItemList` count exists, is at least as large as the HTML list, and the listed count meets or exceeds it;
3. chapter numbers cover every value from 1 through the highest listed number; or
4. the walker reached the terminal TOC page (explicit total page or a next-link chain that ends).

A published latest-chapter number is used as a lower bound: if the list stops below it, the index is rejected even when a terminal page was reached, and the source archive is tried before giving up.

Failed pages, AJAX page failures, the page cap, or any detected pagination that could not reach its terminal page force `complete=false`. A single-page list with none of the above evidence is now rejected rather than assumed complete.

**Diagnostics and job behaviour**

- `NovelChapterIndex.detail` records chapters, pages walked, archive use, and the proof or rejection reason.
- `metadata.json` now contains `indexDetail` beside `indexComplete`.
- `process_epub_download()` fails the job whenever `index.complete == false`, including when source metadata has no chapter count.
- The chapter-index panel displays `index.detail` on rejection.

**Verification**

- `cargo test`: **38/38 passing** (13 new tests).
  - Generic fixtures: NovelFull-style `#idData` + `?page=N` + AJAX, path pagination, next-link pagination, JSON-LD `ItemList`, template chapter items, NovelPing archive discovery, advertised-count scoping, scoped-container selection, no-proof rejection, pagination-query cleanup, JSON archive unwrapping, unrelated JSON-LD list rejection, sampled JSON-LD rejection, and unrelated/WordPress-ID page-link rejection.
  - Export test: an incomplete index writes `"indexComplete": false` and `"indexDetail": "…"`, and `complete` stays false.
- `npm run build`: passing (`tsc --noEmit` + Vite).
- Live QA (`NOVELFORGE_QA_INDEX_URL`):
  - `novelping.com/book/prison-of-eternal-sky` → **53 chapters, complete=true**, `source chapter archive queried; chapters 1–53 are all present.`
  - `divinedaolibrary.com/story/alchemy-master/` → **1,227 chapters, complete=true**, `the source advertises 1,227 chapter(s) and all are listed.` (live page now advertises 1,227; it has one skipped chapter number, so the highest number is 1,228.)
  - `novelfull.net/reverend-insanity.html` → **blocked by Cloudflare HTTP 403** for the app's reqwest/rustls client while `curl` receives HTTP 200. Live walking is deferred to **NF-017 anti-bot/throughput foundation**; the NovelFull markup path is covered by the unit fixture above.

**Known consequence:** single-page sources that expose none of the five proofs are now rejected. That is deliberate honesty, but source-specific parsers (NF-008) may need to add a proof signal for a source that only renders its complete list without a count or pagination marker.

---

## NF-003 — Source search only covers NovelFire + NovelPhoenix

**Status:** `[x] DONE` (2026-11-15)  
**Priority:** P0  
**Area:** `src-tauri/src/resolver.rs`, `src-tauri/src/catalogue.rs`, `NovelSearchPanel.tsx`, `src-tauri/src/commands.rs`

### Evidence

`SEARCH_SOURCES` contains exactly two entries. The UI has a source filter with `all`, `novelfire.net`, `novelphoenix.net`. There is no way to search any of the other 47 enabled searchable lncrawl sources.

### Why it matters

- The user sees “All Sites” but the app really means “the two sites NovelForge knows”.
- Many English translations of Korean/Chinese/Japanese novels live on other lncrawl sources.
- A title that exists on only one of the 47 other searchable sources cannot be found without pasting a URL externally.

### Fix plan

1. Build a source catalogue from `lncrawl sources list -o json`.
2. Store a normalized source descriptor:
   - id/URL
   - display name
   - language
   - search capability
   - manga capability
   - MTL flag
   - enabled/disabled state
   - NovelForge support level
3. Add a **source picker** to webnovel search:
   - “NovelForge verified” group first,
   - “Searchable” group,
   - “Direct URL only” group.
4. Search selected source(s) on demand; do not search all 49 concurrently by default.
5. Add a “Search all searchable sources” advanced action with per-source timeout and clear partial-results reporting.
6. Show which sources failed and why, instead of silently dropping them.

### Acceptance criteria

- [x] A source picker lists every enabled searchable source, plus NovelForge
      direct-URL sources, with language and capability labels.
- [x] Searching a source that is not NovelFire/NovelPhoenix returns real results.
- [x] Search failures are surfaced per source.
- [x] “All Sites” is renamed or redefined so it does not imply universal coverage.
- [x] A source catalogue snapshot is generated reproducibly by a script/test.

### Resolution

Complete (2026-11-15).

**Catalogue.** `src-tauri/resources/lncrawl-sources.json` is a deterministic
snapshot generated by `scripts/build_source_catalogue.py` from
`lncrawl sources list -o json`:

```bash
LNCRAWL=~/.local/share/novelforge/tools/lncrawl-venv/bin/lncrawl
$LNCRAWL sources list -o json | python3 scripts/build_source_catalogue.py \
  > src-tauri/resources/lncrawl-sources.json
```

The snapshot currently holds **339 enabled sources — 49 searchable, 21 manga**.
`catalogue.rs` loads it once, merges each entry with the `sources.rs` adapter
registry (support level and `download_ready`) and exposes `all()`,
`searchable()` and `find_searchable()`. Search still runs against the live
source URL, so a stale snapshot only affects the picker.

**Search.** `search_novels_from()` accepts an explicit source list:

- empty → NovelForge registry defaults (NovelFire + NovelPhoenix),
- `all` → every enabled, searchable, non-manga catalogue source,
- one or more URLs/hosts → exactly those sources,
- a known direct-URL-only host → an explicit per-source failure, never a silent
  empty result.

Sources run at concurrency 4 with a 40 s per-command budget. `SearchResponse`
now carries `searched` (sources that answered) and `failures`
(`SourceSearchFailure { source, label, reason }`). The parser handles titles
and URLs wrapped by the terminal, and `last_error_line()` ignores tqdm
`Searching:` bars so a healthy run is not reported as a failure.

**UI.** `NovelSearchPanel` replaces the old three-value filter with:

- a **source picker** (`NovelForge sources`, `All searchable sources (N)`,
  searchable sources grouped by language, and a `NovelForge direct-URL sources`
  group). The other ~289 non-searchable lncrawl hosts stay reachable by pasting
  a page URL; listing them all would bury the usable entries.
- a dynamic **result-source filter** built from the current results,
- a per-source **failure panel**,
- a direct-only warning that blocks search and points at URL paste.

The indicator now reads `Live search · <source> · N answered` instead of
claiming “All Sites”.

**Direct-only sources.** NovelFull, Divine Dao Library and NovelPing expose no
crawler search, so the picker lists them as direct URL only and the backend
refuses a title search with a specific reason. Their chapter adapters are
tracked separately as NF-019, NF-020 and NF-021 (all done).

**Live evidence.**

| Check | Result |
|---|---|
| `wondernovels.com`, query `beginning after the end` | **1 result**, 0 failures, 8.1 s |
| registry default, query `reverend insanity` | **13 results**, 2 sources searched, 0 failures, 4.1 s |
| `sources=["all"]`, query `reverend insanity` | **43 sources searched, 47 results, 0 failures, 76.2 s** |
| `sources=["novelfull.net"]` | **0 searched, 1 explicit failure**: “This source is direct-URL only; it does not offer title search.” |

Logs: `/tmp/opencode/nf003-search-wondernovels.log`,
`/tmp/opencode/nf003-search-default.log`,
`/tmp/opencode/nf003-search-all.log`,
`/tmp/opencode/nf003-search-direct.log`.

**UI check (2026-11-15).** Launched the dev app and opened Webnovels:

- the picker shows `NovelForge sources`, `All searchable sources (43)` and
  language-grouped hosts,
- selecting `novelfull.net · en` updates the indicator to
  `Live search · novelfull.net` and shows the amber warning
  “novelfull.net is listed by lncrawl for direct links only…”,
- the result-source filter is built from the current results.

Verified live in the Tauri dev window; no persistent screenshot was kept.

**QA harness.** Debug builds accept `NOVELFORGE_QA_SEARCH`,
`NOVELFORGE_QA_SEARCH_SOURCES` (comma-separated, or `all`) and optional
`NOVELFORGE_QA_SEARCH_DEBUG`.

**Tests.** Catalogue parsing/uniqueness and registry-coverage tests added;
wrapped-title/URL parser and progress-bar regressions added. Backend tests
**87/87**, `npm run build` passing.

**Known limit.** `all` searches 43 sources at concurrency 4; a fully dead
internet can take several minutes because each source gets its own budget.
The default picker still searches only the two registry sources.

---

## NF-004 — Full index is fetched twice, with no cache/reuse

**Status:** `[x] DONE` (2026-11-14)  
**Priority:** P1  
**Area:** `NovelSearchPanel.tsx`, `process.rs`, `metadata.rs`

### Evidence

- Selecting a result triggers `api.fetchNovelChapters(selectedUrl)`.
- Starting the download starts `process::start_download`, which fetches the index again.
- There is no shared cache keyed by normalized source URL.
- The index is large: 2,334 entries for Reverend Insanity; the resulting `metadata.json` is about 559 KB.

### Why it matters

- The 24-page NovelFire TOC walk takes roughly 20–30 seconds even when healthy.
- It doubles TOC requests, increasing 429 risk.
- It wastes time and makes the UI feel slower than the download actually is.
- Repeated downloads of the same novel refetch the same index again.

### Fix plan

1. Cache index results by normalized book URL for a configurable TTL (e.g., 6–24 hours).
2. Include ETag/Last-Modified semantics where available.
3. Let the download job accept a pre-fetched index (validated against source URL and timestamp).
4. Add a “Refresh index” button for explicit invalidation.
5. Cache only complete indexes; incomplete/failed indexes should not suppress a retry.
6. Expose cache age in the UI.

### Acceptance criteria

- [x] Downloading a novel whose index is cached skips the TOC walk.
- [x] The cache key is source URL plus source identity, not title.
- [x] A stale/incomplete cache is ignored.
- [x] A manual refresh bypasses the cache.
- [x] No partially fetched index can be reused as complete.

### Resolution

Complete chapter indexes are now cached on disk for 6 hours (override with
`NOVELFORGE_INDEX_TTL_SECS`; cache root override with
`NOVELFORGE_INDEX_CACHE_DIR`):

```text
~/.local/share/novelforge/index-cache/<fnv1a-of-key>.json
```

Implementation details:

- The cache key is `<source host>|<canonical TOC URL>`, never the title.
  Pagination (`page`/`paged`), fragments and NovelFire chapter paths are
  normalised so the picker and the download job land on the same entry.
- Only `complete=true` indexes are written. The stored key is re-checked on read
  to guard against a hash collision, and incomplete entries are ignored.
- Entries carry `fetchedAt`; reads older than the TTL are ignored and the source
  is walked again. Writes use a temp file plus rename so a crash cannot leave a
  half-written entry.
- `fetch_chapter_index` is the cached path used by the download pipeline;
  `fetch_chapter_index_fresh` is the explicit-refresh path.
- The UI shows `cached Nm ago` and a **Refresh** button that calls the fresh
  path, bypassing the cache.
- The timing line now separates `metadata` from `index` so cache hits are
  visible (`index 0.0s`).

### Evidence

- Fresh NovelPhoenix index: 113 chapters, `cached=false`, 3.7 s; cache file
  17,193 bytes.
- Cached read: same 113 chapters, `cached=true`, `age=Some(17)`, **0.0 s**.
- A later process hit the same on-disk entry after 5.6 h
  (`age=Some(20147)`, still inside the 6 h TTL) in **0.0 s**, proving the cache
  survives app restarts.
- Download with a warm cache (chapters 1–2): log
  `Chapter index · 113 chapter(s) from novelphoenix.net · cached 287s ago` and
  `Timing · crawl 45.4s · metadata 2.3s · index 0.0s · audit 0.0s · retry — · export 0.0s · verify 0.0s · import 0.0s · total 47.9s`.
- Manual refresh after a warm cache: `cached=false`, 6.4 s (live TOC walk).
- Stale cache with `NOVELFORGE_INDEX_TTL_SECS=1`: `cached=false`, 4.6 s.
- Cache entry inspected: key `novelphoenix.net|https://novelphoenix.net/mother-of-learning.html`,
  `complete=true`, `total=113`.
- Tests: `cargo test` **49/49**, `npm run build` passing.

---

## NF-005 — Bytes, speed and ETA measure the wrong location

**Status:** `[x] DONE` (2026-11-14)  
**Priority:** P1  
**Area:** `src-tauri/src/process.rs`, `progress_loop`, `useDownloadManager.ts`

### Evidence

`progress_loop` calculates:

```text
bytes = dir_size(destination)
speed = delta(destination bytes) / elapsed
```

But `lncrawl` writes its working files under `~/.lncrawl/novels`. NovelForge imports the finished artifact into `destination` near the end. Destination size therefore stays at 0 (or near 0) during most of the crawl, then jumps during import.

### Why it matters

- The UI often shows `0 B/s` while work is happening.
- ETA is based on a meaningless byte rate.
- Users cannot tell whether the app is stuck, rate-limited, or just waiting for a source.
- It hides the true bottleneck, which is usually request latency and source throttling.

### Fix plan

1. Make progress phase-aware:
   - resolving,
   - fetching index,
   - downloading chapters,
   - retrying,
   - reading/auditing EPUB,
   - exporting structured files,
   - verifying,
   - importing.
2. Track the real work directory:
   - `~/.lncrawl/novels/...` for lncrawl,
   - destination only during import/export.
3. Use chapter counts for primary progress when available.
4. Parse lncrawl’s own percent and chapter messages, but distinguish them from local export progress.
5. Report speed only when meaningful; otherwise show “indexing” or “waiting for source”.
6. Add elapsed time per phase and a final timing summary.

### Acceptance criteria

- [x] No `0 B/s` while lncrawl is actively writing to its work directory.
- [x] UI displays the current phase.
- [x] ETA uses chapter/request progress, not destination bytes.
- [x] Final report includes index time, chapter time and export time separately.
- [x] Pause/resume does not corrupt the ETA calculation.

### Resolution

Every webnovel job now gets a private lncrawl data path:

```text
<download root>/.novelforge-work/<job id>
LNCRAWL_DATA_PATH=<that path>
```

The progress sampler measures `<work dir>/novels` during crawl phases and the job
destination only during import/export/verify. It never copies the global
`~/.lncrawl/config.json`, because that file pins an absolute `database.url`.

Implementation details:

- `JobProgress` shared atomics carry percent, chapter counts, phase and last
  activity from the stdout/stderr pumps to the 900 ms sampler. No job-store lock
  is held while parsing or measuring.
- Speed uses an 8-second byte window; ETA uses a 20-second percent window and
  refuses flat or backwards movement. Paused wall-clock time is excluded from
  elapsed and clears both windows on resume.
- lncrawl disables tqdm when stdout is a pipe, so chapter progress falls back to
  counting real `.zst` chapter files in the work directory. The total comes from
  the selected chapter spec, or from the lncrawl panel `N volumes, M chapters`
  when no selection was made.
- Phase inference: chapter files present → `DownloadingChapters`; all selected
  chapters present with images → `DownloadingImages`; all present without images
  → `Packing`. Output phases are set directly by the pipeline.
- Retries reuse the same work dir and progress state so `--resume`/`--refresh`
  can reuse cached chapters.
- The job work dir is removed after a successful run, including the empty
  `.novelforge-work` container. Failed/cancelled jobs keep it for inspection.
- `list_library` skips hidden path components, so the work tree never appears as
  a library entry, byte count, or novel.

### Evidence

- Live NovelPhoenix `https://novelphoenix.net/mother-of-learning.html`, chapters
  1–8: **Completed** in 130 s, EPUB 365,381 bytes verified. Paused at 38% for 6 s;
  resume kept active elapsed and produced `eta=Some(18)`. Phase sequence:
  `FetchingDetails → DownloadingChapters → Packing → FetchingIndex → Completed`.
- Final timing run, chapters 1–2:
  `Timing · crawl 42.5s · index 5.3s · audit 0.1s · retry — · export 0.1s · verify 0.1s · import 0.0s · total 48.0s`,
  followed by `Removed lncrawl work dir …/.novelforge-work/<job id>`. The work
  tree was absent afterwards.
- Startup latency explained: the app resolver returns instantly for direct URLs
  (`Resolved · Direct source link` precedes the lncrawl launch line). lncrawl then
  spends ~11–12 s on SQLite migrations and its source-sync check, followed by the
  detail page and 3-page TOC fetch before the first chapter file appears. The
  earlier ~105 s `FetchingDetails` sample was source latency inside lncrawl, not
  an app-side resolver wait.
- Library decoy check: a 164 KB fake EPUB plus `metadata.json` under
  `.novelforge-work/decoy/...` did not appear in the response; `count` stayed 1
  and `totalBytes` (345,750) matched the real output tree exactly.
- wbnovel caveat: the pipeline produced 5 chapter files and an 86 KB verified
  EPUB, but the job failed NF-002 completeness (only 20 chapters, no proof), so it
  is not used as NF-005 evidence.
- Tests: `cargo test` **45/45**, `npm run build` passing.

---

## NF-006 — Duplicate output folders for the same novel

**Status:** `[x] DONE` (2026-11-15)  
**Priority:** P1  
**Area:** `duplicates.rs`, `process.rs` destination selection, `export.rs` merge, `commands.rs`, `NovelSearchPanel.tsx`

### Evidence

Each job gets a new UUID suffix, e.g.:

- `reverend-insanity-6d155052`
- `reverend-insanity-da03200a`
- `reverend-insanity-234b51e6`

There is no logic that detects an existing structured folder for the same normalized source URL.

### Why it matters

- Duplicate downloads waste disk and confuse the shelf.
- Full-index metadata is large; duplicates add hundreds of KB or more each.
- The user cannot tell which folder is current.
- Library cards can show the same novel multiple times.

### Fix plan

1. Normalize source URLs (strip tracking/query/fragment; canonicalize trailing slash).
2. Before creating a destination, search the library for the same `sourceUrl`.
3. Offer:
   - update existing,
   - create a separate edition/copy,
   - cancel.
4. Store a stable `bookId` in `metadata.json`.
5. Optionally merge missing chapters into an existing folder instead of duplicating.
6. Migrate old UUID folders by reading `metadata.json` and grouping by source URL.

### Acceptance criteria

- [x] Re-downloading an existing novel prompts for update/copy/cancel.
- [x] Update replaces changed chapter files and preserves existing ones when appropriate.
- [x] Library shows one card per edition, not accidental duplicate folders (new downloads; legacy cleanup is NF-010).
- [x] A test covers same URL with different query strings and trailing slashes.

### Resolution

Duplicate identity is the **normalized source URL**, never the title or folder name.
`export::normalize_source_url` strips fragments, tracking parameters (`utm_*`, `fbclid`,
`gclid`, `ref`, …), a leading `www.`, the `http`/`https` distinction and trailing
slashes, while preserving meaningful queries such as `?p=123`. Query pairs are sorted
so parameter order cannot create a second identity.

`duplicates::find_existing_download` walks the library (depth 4, hidden trees skipped),
reads each `metadata.json`, and compares normalized URLs. It reports the newest
**complete** match (falling back to the newest match when none is complete) plus a
`matches` count. The `find_existing_download` Tauri command exposes this to the studio.
A separate `bookId` was not added: the normalized source URL is already the stable
identity and is present in every `metadata.json` produced so far.

The download flow now fails closed: a webnovel job with no duplicate policy returns
an actionable error instead of creating a second folder. The studio preflights the
URL through `api.findExistingDownload` and shows a modal with **Update existing**,
**Keep both** and **Cancel**, including the folder, downloaded/total chapters,
complete/partial state and matching-folder count.

- **Update** reuses the existing folder. A running job already targeting that folder
  blocks a second update. The export merges the previous `metadata.json`: chapters
  downloaded in earlier runs but absent from the current selection stay referenced and
  on disk; superseded files are removed once nothing references them; the cover is
  preserved when the new run has none.
- **Copy** allocates an explicit `slug-copy-N` edition and leaves the original intact.

Chapter files are now named by **chapter serial**, not EPUB spine position. Before this
change, updating chapters 3–4 would write `0003`/`0004` based on the partial EPUB while
preserved chapters 1–2 already owned those names. Serial naming keeps partial updates
collision-free. Live QA also exposed that `normalize_title("Chapter 1")` collapsed to an
empty key, so chapter 2 matched chapter 1's manifest entry; the fallback now keeps bare
chapter titles distinct, with a regression test.

**Live evidence (NovelPhoenix, `mother-of-learning.html`, 113-chapter index):**

| Scenario | Result |
|---|---|
| Fresh chapters 1–2 | `mother-of-learning-a9de311e`; metadata `2/113`, files `0001`, `0002`; EPUB verified. |
| Repeat without policy | `start failed: “Mother Of Learning” already exists … Choose Update … or Keep both …`; still one folder. |
| Update chapters 3–4 | Same folder; logs `Updating existing structured folder` and `preserving up to 2 downloaded chapter(s)`; metadata `4/113`; files 1–4; new EPUB `Mother Of Learning-2.epub`, old EPUB removed. |
| Copy chapters 1–2 | `mother-of-learning-copy-2`; two folders, two library cards (`4/113` complete and `2/113` complete). |

Caveat: pre-existing accidental duplicate folders from earlier builds are **not**
auto-deleted. The prompt reports `matches` and updates the newest complete match;
cleanup/migration remains NF-010. Tests **54/54**; `npm run build` passing.

---

## NF-007 — No delete/retry/update actions for structured downloads

**Status:** `[x] DONE`  
**Priority:** P1  
**Area:** `LibraryPage.tsx`, `library_actions.rs`, `paths.rs`, `commands.rs`, Tauri commands

### Evidence

The library can reveal and verify an EPUB. Incomplete folders are visible, but there is no first-class action to:

- delete a partial folder,
- retry a specific failed folder,
- update an existing novel,
- open the structured chapter folder,
- view the full chapter index,
- copy the source URL.

### Why it matters

When a job fails, the partial output intentionally remains. Without cleanup/retry actions, failed artifacts accumulate and the user has to use a file manager.

### Fix plan

1. Add `delete_download` with a strict containment check under the download root.
2. Add `retry_download` that reconstructs the request from `metadata.json`.
3. Add `update_download` for complete folders.
4. Add “Open chapters folder” and “View index”.
5. Add confirmation dialogs for destructive actions.
6. Keep the partial folder on failure, but mark it clearly and offer one-click cleanup.

### Acceptance criteria

- [x] Deleting a folder requires explicit confirmation and cannot escape the download root.
- [x] Retry uses the original source URL and chapter selection.
- [x] Update preserves existing chapters unless refresh is requested.
- [x] No raw file manager is required for normal recovery.

### Resolution

Library actions now operate on the exact clicked folder. A new
`library_actions.rs` module provides `details`, `chapter_folder`,
`delete_download`, `retry_download` and `update_download`; `paths.rs` supplies
canonical root/containment checks and hidden-path detection shared with
`commands.rs`. Every action re-validates the folder against the configured
download root, so `..` segments and symlinks cannot escape the sandbox.

`DownloadOptions.targetFolder` is a runtime-only field used by retry/update. It
forces the job to write into the selected folder instead of the duplicate
"newest match" heuristic. `process.rs` re-checks containment, requires a readable
`metadata.json`, compares the folder's normalized source URL with the resolved
source, and refuses a folder already targeted by a non-terminal job.

- **Retry** reads `metadata.json` and limits candidates to the stored
  `selection` written by every new export. It keeps chapters whose manifest
  entry is `downloaded` *and* whose file still exists, and selects the missing
  serials inside that selection (`3,5`, `2-3,5`, …). Older exports without a
  `selection` fall back to the full manifest. If the selection knows of no
  missing chapter it fetches a fresh index and falls back to update selection.
  An incomplete source index is refused with its `indexDetail`.
- **Update** always fetches a fresh complete index. By default it selects only
  chapters absent from the stored manifest, preserving every existing file.
  `refresh=true` selects `1-N` and re-downloads everything. If nothing is
  missing it fails with "Already up to date" and points at Refresh.
- **Delete** refuses the download root, hidden work trees, folders without
  `metadata.json`, and folders targeted by a running job. A successful delete
  emits `library://changed` so the studio refreshes immediately.
- **Chapters / Index** open `chapters/` (falling back to the structured folder
  for older exports) and load the stored chapter list on demand. The index
  viewer shows total/downloaded counts, index completeness, `indexDetail`, and
  each chapter's file/missing state.
- **Studio** card actions: Retry missing or Update, Chapters, Index, Reveal,
  Verify EPUB, Copy source URL, Refresh all (confirmed), Delete (confirmed).
  Partial folders keep a visible note explaining that retry fetches only missing
  chapters and that the folder can be deleted.

The debug harness gained `NOVELFORGE_QA_ACTION=index|details|chapters|delete|retry|update`
plus `NOVELFORGE_QA_FOLDER` and `NOVELFORGE_QA_REFRESH`, sharing the same
`wait_for_job` progress printer as normal downloads.

### Evidence

Live QA used `/tmp/opencode/nf007-downloads` with real NovelPhoenix
`mother-of-learning.html` output and small metadata fixtures.

| Check | Result |
|---|---|
| Details payload | `Retry Fixture` · 4 chapters · 2 downloaded · `indexComplete=true` · range 1..4 · downloaded `[1, 2]` |
| Chapters action | `/tmp/opencode/nf007-downloads/retry-fixture/chapters` |
| Retry missing | Job `57f1a2d3…` completed in 19 s; downloaded only chapters 3–4; chapters 1–2 retained their original fixture text; final folder has `cover.jpg`, `metadata.json`, 4 chapter files and `Mother Of Learning.epub` (`4/113`) |
| Retry respects selection | Fixture with `selection="3-4"` and chapter 3 present: job `d5ff28a9…` completed in 12 s; fetched only chapter 4; chapters 1–3 preserved; chapters 5–113 untouched; the new manifest records `selection=4` for that run |
| Update already current | `Already up to date — every chapter in the current index is downloaded. Use Refresh all chapters to re-download them.` |
| Retry fallback when nothing is missing | Same up-to-date refusal after fetching a fresh index |
| Update missing chapters | Job `603cfd5c…` completed in 23 s; downloaded only 112–113; chapters 1–111 preserved; final `113/113`, `indexComplete=true`, 113 chapter files |
| Delete guards | Refused: plain folder (`not a structured NovelForge download`), download root, outside-root path, hidden `.novelforge-work/decoy` |
| Delete success | `delete-fixture` removed and library refresh event emitted |
| Library payload | All three structured fixtures report `retryable=true`; hidden decoy is absent |
| Busy-folder guard | Unit test: running job blocks the folder; completed job no longer does |

Tests **62/62**; `npm run build` passing. Refresh-all selection (`1-N`) is
covered by the `update_chapter_spec` unit test; a live refresh was not run
because it would intentionally re-download all 113 chapters.

Caveat: `complete` continues to mean "the selected run exported and verified
successfully". A deliberate 1–4 selection therefore still reports
`complete=true` at `4/113`; the library card always shows downloaded/total and
offers Update to fetch the remaining chapters. Deleting is permanent (no trash);
the confirmation dialog is the only guard.

---

## NF-008 — No parser abstraction for expanding to more sources

**Status:** `[x] DONE`  
**Priority:** P1  
**Area:** `metadata.rs`, new source adapter layer

### Core answer to “can we duplicate parsers?”

**No, not by blindly copying one parser.**

Each site can differ in:

- search URL and result markup,
- book-page metadata location (OpenGraph, JSON-LD, custom DOM),
- chapter-list markup,
- TOC pagination style:
  - `?page=N`,
  - `/page/N`,
  - AJAX/JSON endpoint,
  - “load more” button,
  - infinite scroll,
- chapter link format,
- chapter content container,
- anti-bot behaviour and required headers/cookies,
- language and translation policy.

Some sites share a platform, so a **platform profile + small overrides** can cover many sites. Others need a dedicated adapter.

### Recommended architecture

Introduce a source-adapter layer with a declarative profile and optional code hooks:

```text
SourceProfile
  id
  display_name
  hosts[]
  language
  origin_languages[]       // e.g. ["ko"] for Korean-origin English translations
  search:
    supported
    url_template
    result_selector
    title_selector
    link_selector
    cover_selector
  metadata:
    title_selectors[]
    author_selectors[]
    synopsis_selectors[]
    tag_selectors[]
    cover_selectors[]
    chapter_count_selectors[]
  toc:
    url_strategy             // book page, /chapters, custom
    chapter_link_selectors[]
    pagination: query | path | next-link | json | none
    last_page_selector
    total_chapters_selectors[]
  chapter:
    content_selectors[]
    remove_selectors[]
    title_selectors[]
  transport:
    headers
    impersonation_profile
    requires_browser
    rate_limit
    max_concurrency
```

### Engine layers

1. **Platform profiles** for common CMS/templates:
   - WordPress/Madara-style,
   - Blogger,
   - NovelBin-style clones,
   - generic OpenGraph + chapter-link pages.
2. **Site overrides** for the parts that differ.
3. **Generic fallback** that never claims completeness without proof.
4. **Capability flags** so unsupported sources are visible as unsupported, not silently broken.

### Hybrid crawler strategy

- Keep lncrawl for breadth and chapter content where its source adapters already work.
- Add native Rust index adapters for the sources we want first-class.
- For a new source, require:
  - a live search test,
  - a full-index test,
  - a 1–3 chapter export test,
  - a failure test for truncated TOC.

### Acceptance criteria

- [x] Adding a source does not require editing job orchestration.
- [x] A source profile can be validated without a network call.
- [x] Generic fallback cannot mark an unknown source as complete.
- [x] At least one non-NovelFire/NovelPhoenix source passes the full live matrix (FreeWebNovel: complete 2,334-chapter index, verified 1–3 export, truncated-TOC rejection).
- [x] Source support level is visible in the UI.

### Resolution

**Done (2026-11-15).** The registry is the single source of truth for source
capabilities, and FreeWebNovel passed the live matrix.

`src-tauri/src/sources.rs` now owns one `SourceProfile` per host:

- identity (`id`, `display_name`, host suffixes that also match `www.` and
  subdomains), language and origin languages;
- `SupportLevel` (`verified`, `index-verified`, `metadata-only`, `unverified`)
  with evidence text and a `download_ready` flag that is true only for
  `verified`;
- `ParserKind` (`novel-fire`, `novel-phoenix`, `generic`) and capability flags
  for search/metadata/index/chapters;
- `validate()`, which is offline (no network) and rejects empty ids/hosts,
  duplicate hosts across profiles, search capability without a search URL, and
  a generic parser claimed as download-ready.

Wiring changes:

- `metadata.rs` no longer string-matches host names for parser choice, cover
  selection, canonical cache key or index dispatch; it asks the registry. A
  profile that declares `index = false` (Webnovel today) fails immediately with
  a clear “could not be proven complete” message instead of returning a short
  list.
- `resolver.rs` derives its search sources and tie-break ranking from the
  registry instead of a hardcoded `SEARCH_SOURCES` array.
- `list_source_profiles` exposes the registry; Guardrails renders a “Source
  support” table with support level, capabilities and the evidence note, plus
  an `n/total download-ready` counter.

Live regression checks after the switch: NovelPing 53/53 complete (generic
parser, 3.6 s), NovelPhoenix 113/113 complete from 3 TOC pages (dedicated
parser, 4.2 s).

**FreeWebNovel live matrix (2026-11-15).** Added `freewebnovel` to `sources.rs`
(`ParserKind::Generic`, capabilities `search=false, metadata=true, index=true,
chapters=true`) and a matching transport profile (2 safe / 4 max concurrency,
700 ms gap, curl on block).

| Leg | Result |
|---|---|
| Chapter index | **2,334/2,334 complete** from 59 TOC pages in **49.0 s**; detail: `the source advertises 2,334 chapter(s) and all are listed.`; transport `1 retry · 0×429 · 0×503 · 59 curl · 0 blocked · concurrency 4/4`. |
| Download | `--chapters 1-3` completed in **4 m 20 s**: audit `3/3 chapters present and non-empty`, structured export wrote `metadata.json`, 3 `.xhtml` chapters and `cover.jpg`, and the **71,093-byte EPUB verified**. |
| Failure path | Local fixture advertising 10 chapters while listing 3 returned `complete=false`, `3 of 10 advertised chapter(s) are listed.`; the job-level completeness gate is the same one exercised live by Webnovel below. |
| Search | Not declared. lncrawl's inventory lists FreeWebNovel with an empty Search column, so the profile says `search=false` and the UI shows that instead of pretending. Search breadth remains NF-003. |

**Generic metadata author fallback.** FreeWebNovel exposes authors only as
JSON-LD `author[]` and `og:novel:author` (no `meta[name="author"]`), so the
generic parser now reads JSON-LD authors first and falls back to the
`og:novel:author`/microdata chain. This fixed `author: null` on the live export.
A regression test covers both shapes.

**webnovel.com investigation (final answer).** Actual `www.webnovel.com` is
**not supported beyond partial free-chapter downloads**:

- The novel page and `/catalog` are reachable with **curl (HTTP 200)**;
  reqwest receives **Cloudflare HTTP 403**, so the Webnovel transport profile
  now allows the curl fallback. Live metadata resolves `Chrysalis` / `RinoZ`.
- The catalog page contains **877** chapter rows but only **832** chapter
  links; the remaining 45 are locked. lncrawl's adapter explicitly skips locked
  chapters (`# skip locked chapter`).
- The registry therefore keeps `index=false`; a live run downloaded the
  requested chapter into a verified EPUB, then **failed the job** with
  `Full chapter index could not be fetched: Webnovel chapter indexes are not
  supported yet; the source list could not be proven complete.` and kept the
  partial output for inspection.
- Supporting it fully would require authenticated/paywalled chapter access and
  a paid-entitlement-aware catalog, not a parser tweak.

Tests: **82/82** backend (registry, FreeWebNovel transport profile and generic
JSON-LD author cases included). `npm run build` passing. Chapter-content
selectors remain owned by NF-012 (text cleanup), not by source registration.

Evidence logs: `/tmp/opencode/nf008-fwn-index.log`,
`/tmp/opencode/nf008-fwn-download.log`, `/tmp/opencode/nf008-fwn-meta.log`,
`/tmp/opencode/nf008-truncated-index.log`,
`/tmp/opencode/nf008-webnovel-index.log`,
`/tmp/opencode/nf008-webnovel-meta2.log`.

---

## NF-009 — Korean/copyright availability is not represented in UI or data

**Status:** `[x] DONE` (2026-11-15)  
**Priority:** P1  
**Area:** source catalogue, metadata, search UI, docs

### Clarification of the current situation

- There is **no Korean-language (`ko`) crawler** in the installed lncrawl build.
- English-language aggregators may carry English translations of Korean novels.
- NovelFire has displayed notices about not supporting Korean novels for copyright reasons. That means its **catalogue** may intentionally exclude those titles; it does not mean the technical crawler cannot parse a Korean-origin title if one is present.
- The app must not promise that a title exists on NovelFire just because another English source carries it.
- Copyright takedowns and catalogue changes are normal; the app should display source availability honestly.

### Fix plan

1. Add source metadata:
   - site language,
   - origin languages commonly carried,
   - “English translation” badge,
   - copyright/availability note if known.
2. Add search filters:
   - language (English, etc.),
   - source,
   - “translated from Korean” as a **best-effort tag**, not a guarantee.
3. Do not hardcode an “all Korean novels are supported” promise.
4. When a search returns no results, say which sources were searched and that catalogues differ.
5. If the user needs a specific Korean-origin title, support direct URL input to an English translation site that carries it.
6. If raw Korean sites are ever required, that is a separate adapter project; not part of the current English-first scope.

### Acceptance criteria

- [x] Search results show source and language/translation context.
- [x] Empty results explain source coverage instead of implying the book does not exist.
- [x] NovelFire’s catalogue limitations are documented in the app/docs.
- [x] No UI copy claims universal Korean support.

### Resolution

Complete (2026-11-15). Availability is modelled as **evidence-backed registry
data plus best-effort UI tags**, never a promise.

**Registry.** `SourceProfile` gained `availability_note`, surfaced through
`SourceSummary` and the Guardrails source table. NovelFire's origins are now
`["zh", "ja"]` — Korean was removed — with the note:

> NovelFire has removed some Korean-origin novels for copyright reasons; its
> catalogue differs from other English aggregators.

NovelPhoenix, NovelFull, FreeWebNovel and NovelPing keep `ko` in their origin
list because they carry Korean-origin English translations. Direct-URL-only
hosts make no origin claim.

**Catalogue.** `CatalogueSource` now includes `originLanguages`,
`translatedFromKorean` and `availabilityNote`. `translatedFromKorean` is true
only when the site language is `en` and the registry lists `ko`, so unknown
hosts stay untagged. Tests pin NovelFire (no `ko`, note present), NovelPhoenix
(`ko`, tagged) and an unknown host (no claim).

**Search UI.**
- New **origin filter**: Any / Korean-origin (best effort) / Chinese-origin /
  Japanese-origin. Filtering a host with no registry data excludes it rather
  than guessing.
- Result cards show the site language (`EN site`) and a
  `Korean-origin · best effort` chip; hovering explains it is a registry tag,
  not a guarantee.
- The expanded result and the search/failure area show per-source availability
  notes.
- The picker marks registry sources that carry Korean translations.
- Empty results name the sources that answered and explain that catalogues
  differ and that some sources remove titles for copyright reasons. A
  source-filtered empty view suggests clearing the filter instead.

No copy states or implies that all Korean novels are available. Raw Korean
sites remain out of scope; direct English-translation URLs still work.

### Evidence

- Live search probe, `/tmp/opencode/nf009-search.log`:
  - query `I Alone Level-Up` over `novelfire.net,novelphoenix.net`;
  - 12 results, 2 sources answered, 0 failures, 3.9 s;
  - NovelPhoenix returned `I Alone Level-Up` and `Only I Level Up`;
  - NovelFire returned only unrelated titles, demonstrating that its catalogue
    differs rather than that the novel does not exist.
- Earlier probes (`/tmp/opencode/nf009-fire.err`,
  `/tmp/opencode/nf009-phoenix.err`) reproduced the same split for
  `solo leveling` and `omniscient reader`.
- Live UI review (`I Alone Level-Up`, default NovelFire + NovelPhoenix):
  - 12 results, header read `2 answered`;
  - the two NovelPhoenix matches showed `EN SITE` and
    `KOREAN-ORIGIN · BEST EFFORT` chips;
  - the `SOURCE AVAILABILITY` strip showed NovelFire's copyright note;
  - the origin filter offered Any / Korean-origin / Chinese-origin /
    Japanese-origin.
- `cargo test`: **98/98** passed.
- `npm run build`: passed.
- Manual UI review of the source picker, origin filter, result chips,
  availability notes and no-result copy.

---

## NF-010 — Library has old/unstructured folders that never migrate

**Status:** `[x] DONE`  
**Priority:** P1  
**Area:** `commands.rs`, `export.rs`, `LibraryPage.tsx`

### Evidence

The existing `The Tell Tale Heart (1843).epub` folder appears in the raw archive list but not as a structured novel card because it has no `metadata.json`.

### Why it matters

- Old downloads appear as second-class files.
- Users see “2 files” but only 1 novel.
- The library's real coverage is hidden.

### Fix plan

1. Detect archive-only folders with no `metadata.json`.
2. Offer a non-destructive “Adopt into structured library” action:
   - audit EPUB,
   - extract title/author/cover,
   - generate metadata/index where possible,
   - create chapter XHTML files.
3. Never modify the original file without a backup.
4. Mark adopted entries with provenance: `migrated`.
5. For formats where chapter extraction is impossible, show them as “archive only”, not broken novels.

### Acceptance criteria

- [x] Old EPUB folders can be adopted with one click.
- [x] Adoption does not overwrite the original.
- [x] Library summary distinguishes novels, adopted archives and raw files.
- [x] Migration is idempotent.

### Resolution

Implemented as a non-destructive adoption path:

- `ExportMetadata` and `LibraryNovel` gained `provenance`; adopted rows write
  `provenance: "migrated"`, `source: "migrated"` and no `sourceUrl`.
- `export::adopt_epub` audits the EPUB, splits its spine into `chapters/`,
  extracts the cover and writes `metadata.json` beside the file. The original
  EPUB is only read and is never renamed, rewritten or deleted.
- Adoption is idempotent: a second call returns the stored metadata unchanged.
  Existing structured downloads and unreadable `metadata.json` files are
  refused instead of overwritten.
- `list_library` now returns `archives`: loose files with no `metadata.json`,
  each with an explicit adoptability reason. One EPUB in its own folder is
  adoptable; non-EPUB formats, root-level files, multi-EPUB folders and folders
  with existing chapter files are marked archive-only.
- `adopt_archive` validates path containment/hidden paths/running jobs before
  calling the exporter, then emits `library://changed`.
- The Library page adds a migration queue, an **Adopt** action, a **Migrated**
  provenance badge, and summary tiles for downloaded novels, adopted archives,
  loose archive files and library size. Adopted cards do not offer
  retry/update, and their index dialog labels the local EPUB index instead of
  warning about a missing remote TOC.

### Evidence

- `NOVELFORGE_QA_LIBRARY=1` before adoption: 1 novel, 1 archive
  (`The Tell Tale Heart (1843).epub`, `adoptable: true`,
  `Ready to adopt: 1 chapters present and non-empty.`), log
  `/tmp/opencode/nf010-before.log`.
- `NOVELFORGE_QA_ADOPT=…` adopted the old folder: title
  `The Tell-Tale Heart (1843)`, `provenance: "migrated"`, `source: "migrated"`,
  `sourceUrl: null`, `complete: true`, 1/1 chapters, `retryable: false`
  (`/tmp/opencode/nf010-adopt.log`). The folder gained `metadata.json`,
  `cover.jpg`, `chapters/0001 - The Tell-Tale Heart.xhtml` and
  `chapters/style.css`.
- The original EPUB SHA-256 was
  `0bf58f70255635d7e34959df04601f9c00ccc706cc2fc23f0e48d1c8e7088dc9` before
  and after adoption.
- A second adoption left both the EPUB and `metadata.json` byte-identical
  (`/tmp/opencode/nf010-adopt-again.log`), confirming idempotency.
- `NOVELFORGE_QA_LIBRARY=1` after adoption: 2 novels, 0 archives
  (`/tmp/opencode/nf010-after.log`); the final post-cleanup scan is
  `/tmp/opencode/nf010-final-library.log`.
- Live desktop UI: the migration queue showed the loose EPUB with **Adopt**;
  one click produced the toast “Archive adopted”, a second migrated card and
  summary counts 3 novels / 2 adopted / 0 loose
  (`/tmp/opencode/nf010-ui-adopt.png`). The temporary demo folder was removed
  and **Rescan** returned the library to 2 novels / 1 adopted / 0 loose
  (`/tmp/opencode/nf010-ui-final.png`). The migrated index dialog shows
  **local EPUB index** with a gold provenance note and no red missing-remote-
  index warning (`/tmp/opencode/nf010-ui-index.png`).
- Tests **101/101**; `npm run build` passes.

---

## NF-011 — No human-readable full chapter index file

**Status:** `[x] DONE`  
**Priority:** P2  
**Area:** `export.rs`, `metadata.json` generation

### Current state

The full 2,334-entry list is stored in `metadata.json`, but there is no easy human-readable index in the folder.

### Refinement

- Write `index.json` with a compact chapter list (number, title, URL, downloaded, file path).
- Write `chapters.html` or `index.md` for reading/searching outside the app.
- Optionally add an “Index” page to the bundled EPUB that lists every source chapter and marks downloaded ones.
- Keep `metadata.json` as the canonical machine-readable record.

### Acceptance criteria

- [x] Every structured folder contains a readable full index.
- [x] The index shows downloaded vs. not-downloaded chapters.
- [x] The EPUB can be generated with or without the full index page.
- [x] The index is covered by an export test.

### Resolution

`export_epub` and `export_cbz` now call `write_index_files` after merging the
previous manifest, so webnovel and manga folders always receive:

- `index.json` — title, author, source, counts, completeness detail and every
  chapter with `number`, `title`, `url`, `volume`, `downloaded` and `file`.
- `chapters.html` — a standalone page with the title, author, source link,
  downloaded/missing counts, a text filter, a **Missing only** toggle, local
  links for downloaded chapters and a clear **Not downloaded** marker.

`metadata.json` remains canonical; the sidecars are derived from it. Files are
written after `merge_previous_export`, so preserved chapters from earlier runs
are marked downloaded too.

Older structured folders are backfilled best-effort by `library_novel` during
the next Library scan. The bundled EPUB is not modified: lncrawl produces and
verifies it, and the index is a sidecar, so the EPUB is generated with or
without an embedded index page and stays byte-identical. The sidecar leaves the
EPUB spine untouched (adding a spine document would inflate the audited chapter
count).

Evidence (2026-11-15):

- Tests **103/103**; new tests cover index generation, downloaded/missing
  flags, HTML escaping and Library backfill.
- Fresh NovelPing download (`Prison of Eternal Sky`, chapters 1–2): 2/2 audit,
  53/53 complete index, `index.json` 13,211 bytes, `chapters.html` 15,849
  bytes, 2 downloaded / 51 missing, verified 37,982-byte EPUB in 17.2 s. The
  job log now says `Structured export · metadata.json + index.json +
  chapters.html · 2 chapter file(s) · cover.jpg`.
- Real folder `~/Downloads/NovelForge/reverend-insanity-6d155052` (2,334
  chapters, 3 downloaded): `index.json` 557,837 bytes, `chapters.html` 591,063
  bytes, 3 `data-status="downloaded"` and 2,331 `data-status="missing"` rows.
- Adopted folder `11549884-the-tell-tale-heart-1843-0ed5b91c`: 1/1 chapter,
  `index.json` 537 bytes with `source: "migrated"` and the adoption detail.
- `chapters.html` rendered in the Review pane with the correct header
  (`3 of 2334 chapters downloaded · 2331 missing · complete source index`).
- `npm run build` passes.

---

## NF-012 — Chapter text can contain duplicate titles/site chrome/ads

**Status:** `[x] DONE`  
**Priority:** P2  
**Area:** `export.rs`, chapter cleaning

### Evidence

A sampled XHTML chapter begins with the chapter title and then repeats a heading-like sequence:

```text
Chapter 1 - 1: The Heart Of A Demon ...
#1 Chapter 1 - 1: The Heart Of A Demon ...
```

This is likely duplicate title markup from the source.

### Refinement

1. Strip repeated title/heading at the top of the body when it matches the chapter title.
2. Apply a source/site remove-selector list:
   - nav,
   - ads,
   - “next chapter”/“previous chapter”,
   - donation/patreon blocks,
   - translator notes when obvious.
3. Preserve intentional author notes when uncertain.
4. Add a “clean chapter text” test fixture with duplicates and navigation.
5. Do not over-clean: maintain readability and avoid deleting story text.

### Acceptance criteria

- [x] Known duplicate chapter headings are removed.
- [x] Navigation/ad selectors are configurable per source.
- [x] Story text is never silently truncated.
- [x] Regression fixture covers a real exported chapter.

### Resolution

New `src-tauri/src/chapter_clean.rs` cleans each split chapter XHTML before it is
written. The bundled EPUB is never rewritten, and the cleaner falls back to the
original bytes when the input is not UTF-8 or cannot be parsed.

Removed at the top of the body only:

- `aria-hidden` heading markers (`#1`) and numeric-only marker headings,
- a duplicate leading heading (the first visible title is kept),
- a leading paragraph that repeats the title, including the NovelFull style
  `<p><strong>Chapter 1 – The heart…</strong></p>`. This subtitle rule only runs
  when a visible heading already exists, so a chapter whose only title is a bold
  paragraph keeps it.

Chrome removal uses `COMMON_CHROME` (`nav`, `[role=navigation]`, chapter-nav,
prev/next, ad, adsbygoogle, patreon, donation, ko-fi and WordPress sharing
blocks) plus per-source tokens from the new
`SourceProfile::chapter_chrome` field. `novelfire`, `novelfull`, `novelping`,
`freewebnovel`, `novelphoenix` and `divinedaolibrary` declare their extra
selectors in the registry; unknown sources still get the common rules.
`html`, `head`, `body`, `main` and content wrappers (`#chapter`, `#content`,
`#chapter-content`) are protected from every selector.

Translator/author notes and any unrecognised paragraph are preserved. A long
story paragraph that merely mentions the chapter title is kept.

Evidence (2026-11-16):

- Regression fixture `src-tauri/tests/fixtures/tell_tale_heart_chapter_0001.xhtml`
  is a real exported chapter (public-domain text). Cleaning removes the
  `aria-hidden` `#1` marker and keeps every paragraph: before/after paragraph
  text lists are identical and the output keeps `xmlns:epub`.
- Tests **113/113**; new tests cover real-fixture preservation, duplicate
  heading/subtitle removal, chrome removal, per-source selectors, protected
  wrappers, long story paragraphs, a title-only paragraph without a heading, and
  invalid input fallback.
- Live NovelFull 1–2 run: `Chapter 1` exported 86 `<p>` elements against 87 in
  the bundled EPUB; the only removed paragraph was
  `Chapter 1 – The heart of a demon never has regret even in death`, with zero
  `aria-hidden` and zero `<strong>Chapter` markers left. Story paragraphs were
  unchanged. 2/2 audit, verified 50,668-byte EPUB, total 42.6 s.
- `npm run build` passes.

---

## NF-013 — Home “All Sites” means “sources in shelf”, not all sources

**Status:** `[x] DONE`  
**Priority:** P2  
**Area:** `HomePage.tsx`, `ShelfRow`, filter labels

### Evidence

`siteOptions` is derived from the downloaded shelf. “All Sites” therefore means “all sites currently represented by downloads”, not “all supported sources”.

### Refinement

- Rename to “All downloaded sources”.
- Or build the filter from the source catalogue and show which have downloads.
- Add a count next to source names.
- Do not imply source coverage in a library filter.

### Acceptance criteria

- [x] Filter copy is unambiguous.
- [x] Empty source groups are not shown as if they have content.
- [x] Source counts are real.

### Resolution

`HomePage.tsx` now derives the source filter from the downloaded shelf and
labels it honestly:

- `All downloaded sources · N novels` replaces “All Sites”.
- Each source is `host · N novels` (or `Local / imported · N novels` for
  folders with no source URL).
- Counts come from the same `shelf` array that feeds the rows, so an entry only
  appears when at least one downloaded novel exists and the count matches the
  visible rows.
- The select carries `title`/`aria-label` with the full selected label, so a
  narrow window truncates the text visually but still exposes the whole label on
  hover and to assistive tech.
- The source field is slightly wider so the full label fits at normal window
  sizes.
- If the selected source disappears from the shelf, the filter falls back to
  `All downloaded sources` instead of rendering a blank select and an empty
  shelf.

Evidence (2026-11-16, live desktop app against the real library):

- Library: 2 novels — The Tell-Tale Heart (adopted, no source URL) and Reverend
  Insanity (`novelfire.net`).
- Closed filter: `All downloaded sources · 2 novels` (full text, no ellipsis).
- Open filter: `All downloaded sources · 2 novels`,
  `Local / imported · 1 novel`, `novelfire.net · 1 novel`.
- Selecting `novelfire.net · 1 novel` showed only Reverend Insanity and the
  filter label kept its count.
- Selecting `Local / imported · 1 novel` showed only The Tell-Tale Heart.
- `npm run build` passes.

---

## NF-014 — “Premium Edition” label implies a non-existent paid tier

**Status:** `[x] DONE`  
**Priority:** P2  
**Area:** Sidebar, About, marketing copy

### Evidence

The sidebar footer shows “Premium Edition” even though no license, account, payment or premium feature exists.

### Refinement

- Remove the label, or
- Replace with a true build label such as `Local build · v1.0`, or
- Implement the tier before advertising it.

### Acceptance criteria

- [x] No monetisation claim exists without a real product feature.
- [x] The footer label reflects build reality.
- [x] About page matches the footer.

### Resolution

The entire sidebar footer status card was removed, not just the label. The
sidebar now ends with the Settings/Guardrails/About navigation; there is no
“Premium Edition”, “Local build”, version badge or engine-count card. The
unused `version`, `enginesReady` and `enginesTotal` props were dropped from
`Sidebar`, and `App.tsx` stopped passing them.

Nothing in the app now claims a tier, license, account or paid edition. The
About page was already truthful (“no accounts, no cloud, no telemetry”,
“Local-first”, real `v0.1.0` from `get_app_info`) and needed no premium copy
removed; its version chip still comes from the actual build.

Evidence (2026-11-16, live desktop app):

- Sidebar renders Home … About with an empty footer area — no card, no label,
  no version/engine line.
- About page renders normally with the real `v0.1.0` build version and no
  monetisation claim.
- `grep -rn "Premium" src` returns nothing.
- `npm run build` passes.

---

## NF-015 — Test matrix is too small for a multi-source crawler

**Status:** `[x] DONE`  
**Priority:** P2  
**Area:** tests, QA harness

### Coverage when logged (2026-11-15)

- 23 Rust unit tests.
- Live QA against NovelFire.
- Local fixture E2E for failure/retry.
- No frontend integration tests.
- No tests for generic pagination, another lncrawl source, cancellation, duplicate downloads or migration.

### Fix plan

1. Add fixture-driven tests for each parser:
   - single-page TOC,
   - paginated TOC,
   - path-paginated TOC,
   - JSON/AJAX chapter list,
   - advertised count mismatch,
   - blocked/429 response.
2. Add a source smoke matrix:
   - at least 3 first-class sources,
   - one generic WordPress/Madara source,
   - one paginated source,
   - one direct-URL-only source.
3. Add frontend component tests or Playwright tests for:
   - search hydration fallbacks,
   - incomplete-index warning,
   - Home shelf filtering,
   - Library actions.
4. Add cancellation and app-restart tests.
5. Record network timings and 429 counts in QA output.

### Acceptance criteria

- [x] CI runs unit tests and at least one offline integration path.
- [x] Live smoke is opt-in and documented.
- [x] A new source cannot merge without a fixture and a smoke result.
- [x] Regression tests exist for every P0 issue in this file. (NF-003/NF-017 rows were added, see below.)

### Resolution

Three layers now exist, all checked in.

**1. Offline gate — `scripts/check.sh`, mirrored in `.github/workflows/ci.yml`**

| Layer | Command | Result |
| --- | --- | --- |
| Rust unit tests | `cargo test --lib` | 114 passed |
| Rust offline integration | `cargo test --test offline_export` | 2 passed |
| Frontend component tests | `npm test` (Vitest + Testing Library) | 3 passed |
| Frontend typecheck + bundle | `npm run build` | passes |

`src-tauri/tests/offline_export.rs` builds a real EPUB ZIP in a temp dir and drives `audit_epub` + `export_epub` exactly as a download job does. It asserts cleaned chapter XHTML (`aria-hidden` and nav chrome removed, story text intact), `cover.jpg`, `metadata.json`, `index.json` (`indexComplete`), `chapters.html`, the 3-indexed/2-downloaded split, and that an incomplete source index forces `complete: false` while still leaving the partial files on disk. `export` and `model` were made `pub` so the integration test drives the real pipeline rather than a copy.

**2. Parser and transport fixtures**

| Plan item | Test |
| --- | --- |
| single-page TOC | `metadata::tests::parses_toc_page`, `parses_novelfire_detail_page` |
| paginated TOC | `metadata::tests::detects_path_and_next_page_pagination`, `pagination_ignores_unrelated_links_and_wordpress_ids` |
| path-paginated TOC | `metadata::tests::generic_toc_url_drops_only_pagination_query`, `expands_template_chapter_items` |
| JSON/AJAX chapter list | `metadata::tests::parses_json_ld_chapter_item_list`, `parses_novelfull_style_toc_and_ajax`, `reads_ajax_json_fragments`, `discovers_novelping_chapter_archive` |
| advertised count mismatch | `metadata::tests::completeness_requires_real_evidence`, `advertised_total_ignores_author_stats`, `ignores_non_chapter_json_ld_item_lists` |
| blocked / 429 | `transport::tests::blocked_host_fails_fast_instead_of_looping`, `rate_limit_halves_concurrency_and_honours_retry_after`, `fetch_retries_429_with_retry_after` (local TCP server returns 429 then 200) |
| duplicate / migration | `duplicates::tests::*`, `library_actions::tests::*`, `export::tests::*` |

**3. Opt-in live matrix — `scripts/source-smoke.sh`**

Refuses to run without `NOVELFORGE_ALLOW_LIVE=1`, downloads chapters 1–2 per target into a timestamped `/tmp/opencode/` directory and writes one log per source:

| Target | Coverage | Evidence |
| --- | --- | --- |
| NovelPhoenix | first-class, paginated TOC | re-run 2026-11-16: `113 chapters from 3 table-of-contents page(s)`, 2/2 audit, 119,453-byte EPUB, total 17.8s, `5 request(s) · 0 retries · 0×429 · 0×503 · 0 blocked` — `/tmp/opencode/nf015-smoke-20261116-023914/novelphoenix.log` |
| NovelFull | AJAX catalogue + curl transport | re-run 2026-11-16: 2/2 audit, verified EPUB, `1 request(s) · 1 curl · 0 blocked` — `/tmp/opencode/nf015-smoke-20261116-024114/novelfull.log` |
| FreeWebNovel | generic parser on a first-class host | NF-008 live run, 2/2 audit, verified EPUB |
| Divine Dao Library | WordPress/Fictioneer template-free TOC | NF-020 live run, 2/2 audit, verified EPUB |
| NovelPing | direct-URL-only source | NF-021 live run, 2/2 audit, verified EPUB |
| NovelFire | 429/transport profile | index-only probe, existing NF-017 evidence |

**4. Timings and throttling in QA output**

Every QA download now prints a final `[qa] transport report:` line (`requests · retries · 429 · 503 · curl · blocked · avg ms · concurrency · gap · rate`) plus the existing `Timing · crawl / metadata / index / audit / export / verify / total` line, so a smoke log carries its own performance and rate-limit record.

**5. Documented, not automated**

- `docs/QA_MATRIX.md` maps every P0 issue (NF-001, NF-002, NF-003, NF-017) to its regression test, documents the live matrix, and defines the four-part gate for a new source (registry evidence, offline fixture, attached smoke log, fail-closed evidence).
- Cancellation stays a live/manual check: it exercises the OS process supervisor and the work-tree retention rule, which a unit test cannot honestly cover. The recipe is in `QA_MATRIX.md`.
- Frontend coverage in this item goes as far as the Home source filter. Search hydration, the incomplete-index warning, Library actions and restart hydration are logged as **NF-025** rather than claimed here.
- `sources::tests::every_profile_documents_evidence` fails the build if a download-ready profile has no evidence note or a dated live matrix.

`scripts/check.sh` was run after the final edits: 114 + 2 Rust tests, 3 frontend tests, production build all passing.

---

## NF-016 — No disk quota / cleanup policy

**Status:** `[x] DONE`  
**Priority:** P2  
**Area:** settings, library, process

### Why it matters

- A full 2,334-chapter novel can produce thousands of files and a large metadata index.
- Partial folders remain intentionally after failure.
- Multiple retries can multiply disk use.
- lncrawl keeps its own work files under `~/.lncrawl/novels`.

### Refinement

1. Show per-novel disk usage.
2. Add a configurable maximum library size.
3. Warn before large “all chapters” downloads.
4. Offer cleanup of lncrawl work directories after successful import.
5. Offer “delete all partial downloads”.
6. Never delete complete downloads without confirmation.

### Acceptance criteria

- [x] Disk usage is accurate and visible per novel.
- [x] A quota warning appears before a job exceeds the threshold.
- [x] Cleanup actions are explicit and safe.
- [x] Successful import can optionally remove lncrawl working copies.

### Resolution

Three disk trees are now measured separately: complete structured downloads,
incomplete structured folders kept for retry, and per-job crawler work trees
under the hidden `.novelforge-work` container. Sizes are summed from real files
(symlinks are not followed), not estimated from file counts.

**Backend — `src-tauri/src/storage.rs`**

| Concern | Behaviour |
| --- | --- |
| `library_bytes` | Every visible file under the download root, excluding hidden work trees |
| `directory_bytes` | Per-folder size, used for each novel card |
| `work_stats` | Bytes and folder count for `.novelforge-work/<job id>` |
| `quota_block` | `Some(message)` only when a quota is set (`0` = unlimited) and `used >= quota` |
| `cleanup_work_dirs` | Removes abandoned job trees; keeps any id that is queued, running or paused |
| `delete_partial_folders` | Removes only folders whose manifest is incomplete; skips live job folders |

`Settings` gains `maxLibraryBytes` (clamped to 100 TiB, `0` unlimited) and
`cleanupWorkAfterImport` (default on). `LibraryResponse` gains a `storage`
summary (`libraryBytes`, `partialBytes/Folders`, `workBytes/Folders`,
`quotaBytes`, `overQuota`) and each `LibraryNovel` gains `diskBytes`.
`list_library` measures the library **after** the index backfill so a folder's
card size and the total always describe the same files.

The quota gate runs in `process::start_download` **before** the resolver touches
the network. It applies only when a brand-new folder would be created:
`target_folder` retries/updates and `duplicate: update` are recovery paths and
stay available even when the library is already over the limit, as does
deleting output. A “Keep both” copy creates a new folder and is therefore
checked. Nothing is ever deleted to enforce the quota.

Successful imports remove their own work tree when `cleanupWorkAfterImport` is
on and log `Removed work dir …`; failed, cancelled and paused jobs retain it.
Turning the toggle off logs `Kept work dir … (work-file cleanup disabled in
Settings)` and leaves the tree for inspection or a later Library cleanup.

**Frontend**

- **Library** — new *Disk usage* panel with Library / Partial output / Engine
  work files / Free of quota, an over-quota warning, `Clean work files` and
  `Delete partial downloads` buttons, and a confirmation dialog that states the
  affected folder count, bytes and the complete-download exclusion. Each novel
  card shows its own size next to the modification date.
- **Settings** — a library quota field in GB (`0 = unlimited`), the current
  library size, and the “Remove engine work files after import” toggle.
- **Webnovels** — an over-quota banner names the current size and limit, and an
  all-chapters run at or above 300 chapters opens a confirmation showing the
  estimated size (from the library's real bytes-per-chapter average, clamped to
  4–200 KB, with a documented fallback) and the projected library size.

**QA probes**

`NOVELFORGE_QA_STORAGE=report|work|partials|all`, `NOVELFORGE_QA_QUOTA_BYTES`
and `NOVELFORGE_QA_KEEP_WORK=1` drive the same commands from a terminal.

**Evidence (2026-11-16)**

| Check | Result |
| --- | --- |
| Fixture scan `/tmp/opencode/nf016-downloads` | `library=15586 partial=7265 (1 folder) work=4098 (1 folder)`; per-novel `8321` complete, `7265` partial |
| `NOVELFORGE_QA_STORAGE=work` | `removed=1 bytes_freed=4098`; complete folder survived |
| `NOVELFORGE_QA_STORAGE=partials` | `removed=1 bytes_freed=7265`; only the complete folder remained |
| Quota gate, 1-byte quota | Failed before network: `Library quota reached: 8.1 KB used of 1 B…` |
| Live NovelPhoenix 1–2, 100 MB quota | Log `Library storage · 299.3 KB of 100.0 MB quota · 2 chapter(s) selected`; success; `Removed work dir …`; report `library=306485`, work 0 |
| Live NovelFull 1–2, cleanup disabled | `Kept work dir … (work-file cleanup disabled in Settings)`; report `work=1906959 (1 folder)`; cleanup reclaimed 1,906,959 bytes |
| Desktop UI | Library showed the Disk usage panel with real sizes, a per-novel `62.1 KB`, and zero partial/work folders; Settings showed the quota field, current library `1.8 MB` and the cleanup toggle |

Unit coverage: four `storage::tests` (hidden-tree exclusion, quota boundary,
running-job retention for work trees, complete/busy exclusion for partial
folders) plus a `settings::tests` quota clamp/default test and five
`src/lib/storage.test.ts` cases (library average, clamping, fallback, large-run
threshold, quota level). Parallel brand-new jobs can still race past the quota
by one job's size; this is accepted at P2 because the gate is a pre-network
check, not a reservation.

---

## NF-017 — Anti-bot efficiency and adaptive concurrency

**Status:** `[x] DONE` (2026-11-15)  
**Priority:** P0 (design every source adapter around it)  
**Area:** source transport layer, lncrawl configuration, new fetcher

### What the system did before this fix

- NovelFire TOC pages: sequential, ~900 ms gap, 3 attempts, 2 s backoff.
- Generic TOC: one page only.
- Webnovel chapter fetching: lncrawl source pacing, now capped at 2 req/s when throttle is
  on. `runner_concurrency` is the scheduler's concurrent crawl-job count, not chapter
  request concurrency.
- No per-host adaptive rate limiter.
- No shared request cache.
- No Retry-After handling in the app layer.
- Browser fallback can be enabled in lncrawl, which is much slower.

### What “more efficient” should mean

Efficiency is not “send as many requests as possible”. It is:

1. **Avoid redundant work**
   - cache metadata and full indexes,
   - reuse the fetched index for the job,
   - only fetch missing chapters on resume,
   - reuse HTTP connections,
   - skip ads/images unless needed,
   - use JSON endpoints/embedded JSON when available instead of rendering HTML.

2. **Adapt to the source**
   - start at low concurrency,
   - increase only when responses are healthy,
   - back off immediately on 429/503,
   - honor `Retry-After`,
   - apply jitter to retries,
   - use a per-host token bucket.

3. **Avoid the browser**
   - prefer raw HTTP with the correct headers,
   - use browser/headless only when the source requires JavaScript,
   - browser mode is expensive in CPU and latency.

4. **Measure and report**
   - request count,
   - retries,
   - 429 count,
   - average latency,
   - chapters per second,
   - index time vs. chapter time vs. export time.

### Recommended starting policy

These are **starting points**, not entitlements:

| Work type | Per-host concurrency | Global cap | Notes |
|---|---:|---:|---|
| TOC/metadata discovery | 1 | 2 | Sequential; 700–1200 ms gap |
| Chapter HTML/text | 2–3 | 8–10 | Increase only after clean responses |
| Cover images | 1–2 | 4 | Small; don’t burst |
| Manga images | 3–4 | 10–12 | Per-host cap still applies |
| Search requests | 1–2 | 4 | Search is bursty; cache results |

### Adaptive algorithm

```text
start at 2 concurrent requests per host
if 20+ consecutive successes and p95 latency stable:
    increase by 1 up to source ceiling (default 4, max 6 for proven sources)
if 429 or 503:
    halve concurrency immediately
    wait for Retry-After or 15–60 s with jitter
    do not retry the same URL more than 3 times
if 403/Cloudflare challenge:
    stop, report the source as blocked, do not hammer
```

### How many concurrent requests can we send?

There is no universal answer. The source controls it.

- NovelFire TOC: 6-way caused 429; sequential 0.8–0.9 s worked.
- lncrawl default chapter concurrency: 5 in the current user config.
- Safe general default: **2–3 per host** for chapter HTML.
- Proven tolerant sources: **4–6 per host**, never global “max”.
- Always cap globally so one site cannot saturate the machine or the network.

The correct question is not “what is the maximum?” but “what is the maximum this source tolerates without degrading result quality?” Benchmark by ramping and watching 429/5xx rates.

### Acceptance criteria

- [x] Per-host concurrency and rate limits are configurable.
- [x] 429/Retry-After causes immediate backoff, not retries at full speed.
- [x] A source profile records a tested safe concurrency.
- [x] Index caching removes duplicate TOC requests.
- [x] The app reports per-phase timings and 429 counts.
- [x] No source is hammered during a failure loop.

### Resolution

Implemented `src-tauri/src/transport.rs` and routed every app-level metadata,
TOC and cover request through it. The transport owns:

- built-in `SourceProfile`s for NovelFull, NovelFire, NovelPhoenix, Divine Dao
  Library, NovelPing, Webnovel and NovelBin, each with a tested safe
  concurrency, request gap, chapter rate, retry ceiling and evidence note;
- a per-host gate that starts at the profile's safe concurrency, spaces
  requests by kind (TOC ≥ 700 ms, metadata ≥ 500 ms, covers ≥ 150 ms), and
  ramps by one only after 20 consecutive successes with stable latency;
- immediate `429`/`503` backoff: the host limit is halved, a host-wide cooldown
  is set from `Retry-After` (seconds or HTTP date) or 15 s plus jitter, and the
  same URL is retried at most three times;
- Cloudflare `403` detection from `Server`/`cf-ray`/challenge markup. Known
  fingerprint-blocked hosts use the system `curl` transport (NovelFull is
  `curl_mode = Always`); unknown hosts try reqwest first and fall back once.
  If curl is also rejected the host enters a five-minute blocked cooldown
  instead of looping;
- per-host counters for requests, retries, 429, 503, blocked, curl requests,
  curl fallbacks and average latency, exposed through the `get_transport_report`
  command, the Guardrails page, `NovelChapterIndex.transport`, job logs and the
  QA harness.

Settings now accept `sourceLimits: [{ host, concurrency, requestGapMs,
chapterRate }]`; the Settings page has a per-host editor and an override is a
hard ceiling, not a starting point. `lncrawl --rate-limit` reads the same
profile rate instead of a hardcoded `2`. The temporary native-tls/TLS probe was
removed after it confirmed reqwest/native-tls also receives HTTP 403.

Scope limit: the gate is **per host only**. The “global cap” column in the
policy table above is not implemented yet; it stays a design target for a
source-expansion pass (NF-008) where several hosts are fetched in one job.

### Evidence

- NovelFull repro before NF-017: `NOVELFORGE_INDEX_CACHE_DIR=/tmp/opencode/nf017-cache
  NOVELFORGE_QA_INDEX_URL="https://novelfull.net/reverend-insanity.html"
  cargo run` → `Source returned HTTP 403 Forbidden` in 0.2 s.
- TLS probe: reqwest/rustls **403** and reqwest/native-tls **403**; system curl
  **200** over both `--http1.1` and `--http2` with the same headers. The block
  is the client fingerprint, not the request headers.
- Live NovelFull after the transport: 2,334/2,334 chapters complete from 59 TOC
  pages in 67.7 s, 59 curl requests, 0 retries, 0×429/503, concurrency 1/1.
  Range `1..2334`, last chapter `Chapter 2334 (END) - Fang Yuan and Giant Sun
  Fight Star Constellation`.
- Cache reuse: the same URL with `NOVELFORGE_QA_INDEX_CACHE=1` returned
  `cached=true age=6s` in 0.0 s with no new TOC requests.
- Simulated 429 (`Retry-After: 1`) through `NOVELFORGE_QA_TRANSPORT_URL`:
  two requests, one retry, 1×429, 1.60 s total, concurrency halved from 2 to 1.
- Simulated Cloudflare 403: reqwest then curl, two requests total, then
  `Source blocked by Cloudflare … Retry later instead of hammering the source`.
  The host cooldown prevents a failure loop (report: `1 blocked`).
- Per-host override through `NOVELFORGE_QA_SOURCE_LIMITS`: concurrency pinned
  to 1/1, gap 1,234 ms, chapter rate 1.5/s, verified live against the local
  429 server.
- Tests **74/74**; `npm run build` passing.

---

## NF-018 — Resolver relies on English title databases and exact source search

**Status:** `[x] DONE`  
**Priority:** P2  
**Area:** `resolver.rs`, `metadata.rs`, `process.rs`

### Evidence

- Direct source search only queried the two built-in sources.
- Fallback used AniList and Wikipedia title matching.
- No Hangul input normalization, romanisation handling, author search, or source-native synonym table.
- Match threshold was a simple string similarity (`0.72`).

### Refinement

1. Add source-native search results as the primary resolver.
2. Add title aliases/synonyms from source metadata.
3. Add author search.
4. Add Unicode normalization and optional romanisation for Korean/Chinese/Japanese titles.
5. Return “no match” with searched-source details instead of a vague failure.
6. Never auto-select a low-confidence match for a download; require confirmation.

### Acceptance criteria

- [x] Korean-origin English titles can be found by their English title on any supported source.
- [x] Author searches work on sources that support them.
- [x] Low-confidence matches are not silently downloaded.
- [x] Search failures list which sources were queried.

### Resolution

The resolver is still **source-native first**, but each stage now returns
evidence instead of a bare title:

| Stage | Behaviour |
| --- | --- |
| Direct URL | Confidence `1.0`, `matched_by=url`, no network search |
| Source search | Existing selected sources (default NovelForge registry; `all` or one explicit catalogue host are still honoured), best title score `0.72` |
| Detail verification | Reads up to eight candidates, round-robin across sources, for the real title, advertised aliases and author (`metadata::fetch_novel_summary`, no cover download) |
| Alias match | JSON-LD `alternateName` or labelled “Alternative names/Also known as” rows, threshold `0.8` |
| Author mode | Title/Author toggle or `author:`/`by:` prefix; the author field must score `0.7` |
| Title databases | AniList (English, romaji, **native** Hangul/CJK, synonyms) then Wikipedia; queried only after source-native matching fails |
| No match | Error names every consulted source, including AniList/Wikipedia when they were reached |

`ResolvedTitle` now carries `confidence`, `matchedBy`, `searched` and up to five
`alternatives`; `NovelMetadata` carries `aliases`. The studio shows the score and
match type, the consulted sources and clickable alternatives. Matches below the
shared `CONFIRM_THRESHOLD` (`0.85`) render a review warning, and
`process::start_download` refuses a non-URL webnovel input below that score
before any folder is created. A clicked URL or alternative always resolves at
`1.0`, so the UI’s explicit confirmation path is unaffected.

Unicode folding lower-cases first, then folds full-width ASCII, ideographic
spaces, combining marks and common Latin diacritics, so “Pokémon”, “Pokemon” and
a full-width form compare equal. Hangul and CJK are preserved for the AniList
`native` title step rather than guessed at.

Live QA found that NovelFull’s fuzzy search can fill the first candidates with
unrelated novels, hiding the real author hit on NovelPhoenix. Candidate
inspection therefore interleaves sources (first hit from every source, then
seconds) and ties break on original search rank, then registry rank.

**Evidence (2026-11-16)**

| Check | Result |
| --- | --- |
| Title `reverend insanity` | `confidence=1.00 matched_by=title` on novelfire.net; alternatives include the NovelPhoenix mirror and four sequels; `searched=["novelfire.net","novelphoenix.net"]` |
| Alias/acronym `master of gu` | AniList synonym → `Reverend Insanity` at `1.00`, searched four sources (AniList, Wikipedia, NovelFire, NovelPhoenix) |
| Author `Gu Zhen Ren` (`mode=author`) | `Reverend Insanity` on novelphoenix.net, `confidence=1.00 matched_by=author`; alternative “Mysteries of Immortal Puppet Master” |
| `author: Gu Zhen Ren` (`mode=auto`) | Same result via prefix parsing |
| No match `qwzxv plorbnak zzz` | `No source link found … Searched: AniList, Wikipedia, novelfire.net, novelphoenix.net` |
| Low-confidence download `reverend insanity 2` | Resolved `Reverend Insanity` at `78%`; `start_download` refused with “Resolve it in Webnovels and confirm the source…” and created no folder |
| Desktop UI (fullscreen) | Author panel showed `100% author match`, `Searched 2 sources`, the alternative button; `reverend insanity 2` showed `REVIEW RESOLVED MATCH · 78% title match` and the confirm warning; clicking the NovelPhoenix alternative switched the source and cleared the alternatives |

Unit coverage: `resolver::tests::interleaves_candidates_across_sources`,
`folds_diacritics_and_full_width_forms`, and
`metadata::tests::parses_aliases_from_json_ld_and_labelled_rows`, plus five
`src/lib/resolver.test.ts` confidence cases.

**Known limits:** alias metadata only exists where the source advertises it
(JSON-LD or a labelled row); NovelPhoenix/NovelFire do not, so those pages rely
on AniList or the title. AniList covers Korean webnovels unevenly, and no
romanisation library is bundled. Both are recorded in the roadmap rather than
claimed as support.

---

## NF-019 — NovelFull chapter adapter is stale (AJAX catalogue)

**Status:** `[x] DONE` (2026-11-15)  
**Priority:** P1  
**Area:** external lncrawl `sources/en/n/novelfull.py`, `scripts/install-tools.sh`, `src-tauri/src/sources.rs`

### Evidence

- `https://novelfull.net/` is enabled in lncrawl, English, `Search: false`.
- The full index is proven complete: **2,334/2,334** chapters from 59 TOC pages
  in 67.7 s via the curl transport profile (NF-017), and cached repeats are
  instant (NF-004).
- Chapter download fails with
  `LNException: Failed to parse chapter list`.
- The live site serves its chapter list through AJAX: `#truyen-id` plus
  `#total-page` and a paged endpoint.
- lncrawl's `NovelFullTemplate` still expects `#rating[data-novel-id]` and
  `ul.list-chapter`.
- reqwest (rustls and native-tls) receives **403**; curl receives **200**, so
  the source profile already uses `curl_mode = Always` at concurrency 1.

### Why it matters

NovelFull is one of the three user-selected expansion sources and is the
largest catalogue of the three. Its index and metadata already work; only the
chapter adapter blocks `verified` status.

### Fix plan

1. Add a NovelFull override in the lncrawl source adapter that reads
   `#truyen-id` / `#total-page` and fetches the AJAX chapter endpoint.
2. Keep the transport profile at `curl_mode = Always`, concurrency 1.
3. Prove a full download on a small selection first, then a full novel.
4. Promote to `verified` only after cover, `metadata.json`, chapter XHTML,
   full index and EPUB all pass.

### Acceptance criteria

- [x] A 3-chapter selection downloads, audits and exports on NovelFull.
- [x] `metadata.json` records `indexComplete=true` with the full 2,334-chapter index.
- [x] The cover and EPUB verify.
- [x] `sources.rs` moves NovelFull from `index-verified` to `verified`.

### Resolution

Complete (2026-11-15).

**Adapter.** `sources/en/n/novelfull.py` now handles the current site instead of
the retired template selectors:

- reads `#list-chapter[data-list-url]`, `#truyen-id` and `#total-page`;
- fetches every catalogue page from
  `/ajax-chapter-list?novelId=<id>&page=<n>` as JSON and parses the returned
  `<li>` rows with `a.con[href]`;
- raises on an empty page or when the collected count is below
  `data-total-chapters`, so a partial catalogue can never look complete;
- falls back to `NovelFullTemplate` when the AJAX markers are absent, which
  keeps the legacy `novelfull.com` markup working.

The patch is repository-owned and reproducible:

- `scripts/patches/lncrawl-v4.14.0-novelfull-ajax.patch`
- `scripts/apply-lncrawl-patches.sh` (idempotent; verifies with `git apply --check`)
- `scripts/install-tools.sh` now installs lncrawl **from source** at the pinned
  `v4.14.0` tag and applies the patch, because the release binary cannot be
  patched. README prerequisites and paths were updated accordingly.

**Promotion.** `sources.rs` marks NovelFull `verified` with
`live_matrix = 2026-11-15`; the transport profile stays `curl_mode = Always`,
concurrency 1, gap 900 ms and chapter rate 2/s.

### Evidence

- Direct lncrawl probe (`/tmp/opencode/nf019-lncrawl.log`): the adapter logged
  `NovelFull catalogue: 2334 chapter(s) from 59 page(s)`, resolved
  `24 volumes, 2334 chapters`, downloaded chapters 1–3 and wrote a 60.1 KB EPUB.
- End-to-end QA (`/tmp/opencode/nf019-qa.log`):
  - destination `/tmp/opencode/nf019-qa/reverend-insanity-37824c07`;
  - `Chapter index · 2334 chapter(s) from novelfull.net · 2334 chapter(s) from
    59 table-of-contents page(s); the source advertises 2334 chapter(s) and all
    are listed.`;
  - `Chapter audit · 3/3 chapters present and non-empty`;
  - `Structured export · metadata.json · 3 chapter file(s) · cover.jpg`;
  - `Success — 1 verified output file(s)`; timing
    `crawl 1m 02.4s · metadata 1.0s · index 1m 00.5s · audit 0.0s · export
    0.1s · verify 0.0s · total 2m 03.9s`;
  - transport `61 request(s) · 0 retries · 0×429 · 0×503 · 61 curl · 0 blocked`.
- Output tree: `cover.jpg`, `metadata.json`, `chapters/0001…0003*.xhtml`,
  `chapters/style.css`, `Reverend Insanity   Novelfull.epub` (61,523 bytes).
- `metadata.json`: `chapterCount=2334`, `downloadedChapters=3`,
  `selection="1-3"`, `indexComplete=true`, `complete=true`,
  `verified.ok=true`, `verified.detail="3/3 chapters present and non-empty"`.
- Patch reproducibility: the patch applies cleanly to a fresh `v4.14.0` clone
  and compiles; a clean editable install from that clone reports
  `Lightnovel Crawler v4.14.0`; both the first application and a re-run of
  `scripts/apply-lncrawl-patches.sh` succeed.
- Tests **93/93** pass; `npm run build` passes; `bash -n` passes for both
  installer scripts.

**Metadata limitation (fixed by NF-023).** At NF-019 time the export title
inherited the page suffix (`Reverend Insanity | Novelfull`) and `author` was
`null`, because the generic parser preferred `og:title` and NovelFull exposed
the author only in `.m-info .item a[href*="/author/"]`. NF-023 added a
source-specific parser and an adapter patch, so current downloads use
`Reverend Insanity` and `Gu Zhen Ren`. The chapter/index/EPUB acceptance above
was unaffected.

---

## NF-020 — Divine Dao Library adapter is stale (site redesign)

**Status:** `[x] DONE` (2026-11-15)  
**Priority:** P1  
**Area:** external lncrawl `sources/en/d/divinedaolibrary.py`, `scripts/install-tools.sh`, `src-tauri/src/sources.rs`

### Evidence

- `https://www.divinedaolibrary.com/` is enabled in lncrawl, English,
  `Search: false`.
- The full index is proven complete: **1,227/1,227** chapters.
- Chapter download fails with `ServerError: Error(404): Novel has no title`.
- The site was redesigned: the novel page now uses `article.story__article`
  and `h1.story__identity-title`; the old adapter queries the previous layout.

### Why it matters

Divine Dao Library is one of the three user-selected expansion sources. Only
the external chapter adapter blocks `verified` status.

### Fix plan

1. Update the lncrawl adapter selectors for `article.story__article` and
   `h1.story__identity-title`, including the chapter list container.
2. Verify the AJAX/paged chapter endpoint if the new theme uses one.
3. Run a 3-chapter live download, then a full matrix.
4. Promote to `verified` only after cover, metadata, chapters, index and EPUB pass.

### Acceptance criteria

- [x] A 3-chapter selection downloads, audits and exports on Divine Dao Library.
- [x] `metadata.json` records `indexComplete=true` with the full index
      (**1,229** chapters at test time; the source grew from 1,227).
- [x] The cover and EPUB verify.
- [x] `sources.rs` moves Divine Dao Library from `index-verified` to `verified`.

### Resolution

Complete (2026-11-15).

**Adapter.** `sources/en/d/divinedaolibrary.py` now targets the Fictioneer
theme instead of the retired layout:

- title from `h1.story__identity-title` (legacy `article header .entry-title`
  kept as a fallback);
- cover from `og:image`;
- author from `.story__identity-meta a.author` / `a[rel="author"]`, with the
  old `Author:` heading as a fallback;
- synopsis from `.story__summary p`; genre/tag links populate `novel_tags`;
- chapter list from `a.chapter-group__list-item-link[href]`, deduplicated and
  kept in page order (the full 1,229-chapter TOC is on the story page, so no
  pagination requests are needed);
- chapter body from `#chapter-content p`, with a text fallback when the theme
  changes again.

The patch is repository-owned and applied by the same installer path as
NF-019:

- `scripts/patches/lncrawl-v4.14.0-divinedaolibrary-redesign.patch`
- `scripts/apply-lncrawl-patches.sh` now applies both source patches.

**Promotion.** `sources.rs` marks Divine Dao Library `verified` with
`live_matrix = 2026-11-15`. The generic index parser still proves completeness
from the story page.

### Evidence

- Direct lncrawl probe (`/tmp/opencode/nf020-lncrawl.log`):
  `Novel title: Alchemy Master`, `Novel author: Silavin`,
  `Novel chapters: 1229`, `13 volumes, 1229 chapters`; chapters 1–3 produced an
  84.2 KB EPUB with three non-empty chapter XHTML files.
- End-to-end QA (`/tmp/opencode/nf020-qa.log`):
  - destination `/tmp/opencode/nf020-qa/alchemy-master-8a30d014`;
  - `Chapter index · 1229 chapter(s) from divinedaolibrary.com · 1229
    chapter(s); the source advertises 1229 chapter(s) and all are listed.`;
  - `Chapter audit · 3/3 chapters present and non-empty`;
  - `Structured export · metadata.json · 3 chapter file(s) · cover.jpg`;
  - `Success — 1 verified output file(s)`; timing
    `crawl 7.7s · metadata 2.2s · index 2.2s · total 12.3s`;
  - transport `3 request(s) · 0 retries · 0×429 · 0×503 · 0 blocked`.
- Output tree: `cover.jpg` (400×600 JPEG), `metadata.json`,
  `chapters/0001…0003*.xhtml`, `chapters/style.css`,
  `Alchemy Master.epub` (86,212 bytes).
- `metadata.json`: `title="Alchemy Master"`, `author="Silavin"`,
  `chapterCount=1229`, `downloadedChapters=3`, `selection="1-3"`,
  `indexComplete=true`, `complete=true`, `verified.ok=true`.
- Patch reproducibility: the DDL patch applies cleanly to a fresh `v4.14.0`
  clone and compiles; a clean run of `scripts/apply-lncrawl-patches.sh` applies
  both NovelFull and Divine Dao Library patches, and a re-run skips both.
- Tests **93/93** pass; `npm run build` passes; `bash -n` passes for both
  installer scripts.

---

## NF-021 — NovelPing has no lncrawl source adapter

**Status:** `[x] DONE` (2026-11-15)  
**Priority:** P1  
**Area:** external lncrawl `sources/en/n/novelping.py`, `scripts/install-tools.sh`, `src-tauri/src/sources.rs`, `src-tauri/resources/lncrawl-sources.json`

### Evidence

- `https://novelping.com/` was **absent** from lncrawl's 338-source enabled
  inventory.
- NovelForge's generic index parser already proved the TOC **53/53**.
- There was no lncrawl chapter adapter, so a download could not be delegated to
  the crawler.

### Why it matters

NovelPing is one of the three user-selected expansion sources. Its small
catalogue makes it the cheapest source to promote if an adapter is added.

### Fix plan

1. Decide between a native NovelForge chapter fetcher and an lncrawl source
   contribution.
2. If lncrawl: add a `novelping.py` source with search disabled, TOC and
   chapter selectors; keep the curl transport profile if needed.
3. Prove a 3-chapter live download and then the full matrix.
4. Register the source in `sources.rs` and promote only after the full matrix.

### Acceptance criteria

- [x] NovelPing appears in the catalogue with a real support level.
- [x] A 3-chapter selection downloads, audits and exports.
- [x] `metadata.json` records `indexComplete=true` with 53/53 chapters.
- [x] The cover and EPUB verify.

### Resolution

Complete (2026-11-15).

**Adapter.** Added `sources/en/n/novelping.py`. The story page renders only the
first 30 chapters, so the adapter:

- reads the novel id from `#rating[data-novel-id]`;
- fetches `/ajax/chapter-archive?novelId=…`, which returns all 53 chapters in a
  `<template data-chapter-item-template>`;
- raises `LNException` when the archive has no chapters, instead of silently
  accepting a short list (note that `PageSoup.select` cannot cross a
  `<template>` boundary in bs4 4.15, so the template is selected first and its
  subtree queried);
- takes title/cover/author from `og:novel:*`/`og:image`, the synopsis from
  `#novel-description-content`, tags from `/novelping-genres/`, and the body
  from `#chr-content p`;
- leaves `can_search = False` because the site has no search endpoint exposed
  to the crawler.

The adapter is repository-owned:

- `scripts/patches/lncrawl-v4.14.0-novelping-adapter.patch`
- `scripts/apply-lncrawl-patches.sh` applies all three source patches and
  compiles all three adapters.

**Catalogue.** Regenerated `src-tauri/resources/lncrawl-sources.json` from the
live `lncrawl sources list -o json`; the only change is the new
`https://novelping.com/` entry (338 → 339 enabled sources, still 49 searchable).
The picker therefore lists NovelPing as a direct-URL-only source.

**Promotion.** `sources.rs` marks NovelPing `verified` with
`live_matrix = 2026-11-15`.

### Evidence

- Direct lncrawl probe (`/tmp/opencode/nf021-lncrawl.log`):
  `Novel title: Prison of Eternal Sky`, `Novel author: Riker_Rhodes`,
  `Novel chapters: 53`, `1 volumes, 53 chapters`; chapters 1–3 produced a
  49.8 KB EPUB with three non-empty chapter XHTML files.
- End-to-end QA (`/tmp/opencode/nf021-qa.log`):
  - destination `/tmp/opencode/nf021-qa/prison-of-eternal-sky-71639578`;
  - `Chapter index · 53 chapter(s) from novelping.com · 53 chapter(s); source
    chapter archive queried; chapters 1–53 are all present.`;
  - `Chapter audit · 3/3 chapters present and non-empty`;
  - `Structured export · metadata.json · 3 chapter file(s) · cover.jpg`;
  - `Success — 1 verified output file(s)`; timing
    `crawl 6.8s · metadata 1.4s · index 1.2s · total 9.5s`;
  - transport `3 request(s) · 0 retries · 0×429 · 0×503 · 0 blocked`.
- Output tree: `cover.jpg` (150×200 JPEG), `metadata.json`,
  `chapters/0001…0003*.xhtml`, `chapters/style.css`,
  `Prison Of Eternal Sky.epub` (50,960 bytes).
- `metadata.json`: `title="Prison Of Eternal Sky"`,
  `author="Riker_Rhodes"`, `chapterCount=53`, `downloadedChapters=3`,
  `selection="1-3"`, `indexComplete=true`, `complete=true`,
  `verified.ok=true`.
- Patch reproducibility: applies cleanly to a fresh `v4.14.0` clone and
  compiles; `scripts/apply-lncrawl-patches.sh` applies all three patches on a
  clean checkout and skips them on a re-run.

---

## NF-022 — Manga engine `kotatsu-dl` is archived; evaluate a maintained replacement

**Status:** `[x] DONE (2026-11-15)`  
**Priority:** P1  
**Area:** `src-tauri/src/tools.rs`, `src-tauri/src/process.rs`, `src-tauri/src/settings.rs`, `src-tauri/src/export.rs`, `src-tauri/src/resolver.rs`, `MangaPage.tsx`, `SettingsPage.tsx`, `AboutPage.tsx`

### Evidence

- `KotatsuApp/kotatsu-dl` is **archived**; latest release **v0.8
  (2025-07-31)**, latest commit 2025-07-31.
- `mikf/gallery-dl` is active: **20,516 stars**, latest release **v1.32.20
  (2026-11-14)**, latest commit 2026-11-14, GPL-2.0, **387 supported sites**
  including MangaDex, MangaKakalot, MangaPark, Naver Webtoon, WEBTOON, Tapas,
  Comick and MangaFire.
- gallery-dl has no unified cross-site search/catalogue; it is URL-based.
- Current NovelForge manga integration is a thin CLI adapter:
  `tools.rs` resolves the binary/JAR → `process.rs::build_args` adds
  `--dest/--format/--chapters/-j/--throttle` → `process_manga_download` audits
  one CBZ and calls `export::export_cbz`.

### Why it matters

An archived engine means every site change requires a manual NovelForge patch.
The user explicitly requires an actively maintained, cross-platform engine with
future multi-platform scope (Android, macOS, Windows, Linux; iOS out of scope).

### Evaluation

| Candidate | Maintenance | Sources | Fit with current architecture | Verdict |
|---|---|---|---|---|
| `kotatsu-dl` | Archived 2025-07-31 | Small, bundled | Exact current fit | **Replace** |
| `gallery-dl` | Very active, releases weekly | 387 sites, URL-based | CLI-in/CLI-out like kotatsu; different archive shape | **Recommended** |
| Suwayomi/Tachidesk | Active server (8,066 stars, pushed 2026-11-12, MPL-2.0) | Hundreds via Tachiyomi extensions | Requires a long-running Java server, extension install and GraphQL client | Too heavy for this iteration |

**Live check.** gallery-dl v1.32.20 was installed in a throwaway venv and run
against a MangaDex title:

- `--chapter-range 1-2 --cbz` produced **one CBZ per chapter** under
  `gallery-dl/mangadex/<title>/`.
- Without `--cbz`, the same run produced per-chapter directories
  (`gallery-dl/mangadex/<title>/v01 c001/`) with numbered images.
- `-D <dir>` forces a **flat** layout and a single `<dir>.cbz`, which would
  collapse chapters into one group. Do not use `-D` for manga.

**What changes.** The difference from kotatsu-dl is the archive shape, not the
engine contract:

1. `tools.rs`: rename `kotatsu_path`/`kotatsu` to `gallery_dl_path`/`gallery_dl`
   (with a one-time settings migration); probe `gallery-dl --version`.
2. `process.rs::build_args`: manga branch becomes
   `gallery-dl --chapter-range <spec> --write-info-json <url>` with a work
   directory, no `-D`, and a `--range`/`--chapter-filter` mapping.
3. `process_manga_download`: accept a **directory of chapter folders** and zip it
   into one CBZ before the existing `audit_cbz` → `export_cbz` path. This is one
   small archive function; the audit/export/metadata layers stay unchanged.
4. `list_manga_sources`: replace `kotatsu-dl --sources` with a catalogue built
   from `gallery-dl --list-extractors` (the same snapshot pattern as NF-003),
   restricted to known manga hosts.
5. Settings/About/Engine metadata: update labels and the upstream link.

Estimated surface: **5–6 files**, no change to the webnovel or books pipeline.
That is a contained adapter swap, not an architecture rewrite.

### Recommendation

**Replace kotatsu-dl with gallery-dl.** It is the only candidate that keeps the
current “resolve binary → build args → audit archive” shape, is actively
maintained, and covers 387 sites on all four target platforms. Suwayomi is the
stronger long-term source ecosystem but needs a server process and extension
management, which is a larger change.

### Acceptance criteria

- [x] `gallery_dl_path` is configurable and detected in Guardrails.
- [x] A MangaDex chapter selection downloads and merges into one CBZ.
- [x] Chapter folders, page counts and cover survive the existing audit.
- [x] The manga source list is derived from gallery-dl, not kotatsu.
- [x] Settings migration keeps an existing kotatsu path from breaking startup.
- [x] `kotatsu-dl` references are removed from docs and UI.

### Resolution

Implemented on 2026-11-15 after user approval.

**Backend**

- `settings.rs`: `gallery_dl_path` replaces `kotatsu_path` (accepts the old
  `kotatsuPath` key once), `manga_language` (default `en`, blank = all
  translations), `manga_format` is `cbz`/`zip` only, and `java_path` and
  `manga_jobs` are gone.
- `tools.rs`: resolves/probes `gallery-dl --version`; `ToolsReport` is now
  `lncrawl`, `gallery_dl`, `aget` (3 engines). Java resolution was deleted.
- `process.rs`: manga jobs get a private work directory. gallery-dl runs with
  `--child-filter`, `--no-input`, `--no-colors`, `--option lang=<code>`,
  `--write-info-json`, and `--sleep-request 1.0` when throttled. The progress
  loop measures `gallery-dl/` and counts one chapter per image-bearing folder.
- `--child-filter` maps the UI chapter numbers to gallery-dl’s own `chapter`
  field; `--range`/`--child-range` are index-based and would have selected the
  wrong chapters.
- Opaque UUID titles (MangaDex) are replaced with the series folder name from
  the downloaded tree before packaging, so folders are `oshimai-<job>` rather
  than a UUID.
- `export.rs`: `package_manga_directory` groups gallery-dl images by chapter
  folder, drops alternate uploads of the same `(lang, volume, chapter)` keeping
  the one with the most pages, and writes one CBZ/ZIP. The existing
  `audit_cbz` → `export_cbz` path then produces `cover.*`, `metadata.json` and
  per-chapter image folders.
- `process_manga_download` reads gallery-dl’s `info.json` instead of scraping
  the source page. The old metadata fetch returned a Cloudflare interstitial
  title (“Browser verification”) on MangaDex; metadata is now the extractor’s
  own title, author, tags, description and language.
- `resolver.rs`: `list_manga_sources` parses `gallery-dl --list-extractors`
  and keeps `Subcategory: manga`. Live result: **26 extractors**, all labelled
  `Multi`, with adult categories tagged for the UI filter.

**Frontend**

- Settings: gallery-dl path, manga language, CBZ/ZIP only; Java and parallel
  jobs removed.
- Manga page: gallery-dl wording, no jobs control, extractor catalogue, adult
  filter driven by extractor tags.
- About/Guardrails/engine metadata: gallery-dl labels, 3-engine fallback.
- `scripts/install-tools.sh` now creates a gallery-dl venv and wrapper instead
  of downloading the Kotatsu JAR; README prerequisites and troubleshooting no
  longer require Java.

**Live evidence**

- Extractor catalogue: **26 manga sources** in
  `/tmp/opencode/nf022-sources.log`.
- End-to-end MangaDex run (`chapters=1`, `/tmp/opencode/nf022-e2e2.log`):
  `--child-filter "chapter is not None and str(chapter) in ('1', '1.0')"`,
  4 volume-1 chapters / 27 pages, one `Oshimai.cbz` (27,124,747 bytes), cover
  1061×1500 PNG, `metadata.json` with real title/author/tags, folder
  `oshimai-9773e9a2`, verification `4 chapter folder(s) · 27 page(s)`, total
  24.6 s.
- Earlier `chapters=1-2` run confirmed multiple volume/chapter matches and
  produced one archive with chapter folders
  (`/tmp/opencode/nf022-e2e.log`).
- Fail-closed check (`chapters=9999`, `/tmp/opencode/nf022-empty.log`):
  gallery-dl matched no chapters, packaging failed with “no download tree”,
  the job ended `Failed`, and the work tree was retained for inspection.
- Synthetic tests cover CBZ packaging, alternate-upload dedupe, empty-tree
  rejection, info.json metadata, manga chapter filter and manga progress
  counting. **93/93** Rust tests pass; `npm run build` passes.

**Known limitation:** gallery-dl has no parallel chapter pool, so
`manga_jobs` was removed rather than faked. Throttling is a request delay.

---

### 4.1 Do not create 338 copy-pasted parsers

A parser is not just “get chapter links”. For each source you need to answer:

1. How do I search?
2. How do I identify the book page?
3. Where are title/author/tags/synopsis/cover?
4. Where is the real TOC?
5. How does the TOC paginate?
6. How do I know I reached the end?
7. Where is chapter content?
8. What must be removed?
9. What anti-bot profile is safe?
10. Does the source require login/browser?

Copying a parser from one site to another usually breaks on points 2–7.

### 4.2 Use platform profiles

A large fraction of English webnovel sites use a small number of CMS/templates. Build profiles first:

- WordPress/Madara-style novel themes,
- Blogger/Blogspot serials,
- NovelBin-style clones,
- generic OpenGraph book pages,
- custom sites with a simple chapter list.

Then add per-site overrides only where the profile fails.

### 4.3 Source tiers

| Tier | Meaning | Requirements |
|---|---|---|
| Tier A | NovelForge verified | Search + full TOC + chapter content + export + live smoke |
| Tier B | Searchable, index supported | Search + full TOC + chapter export |
| Tier C | Direct URL only | lncrawl can crawl; no NovelForge search/index promise |
| Tier D | Unsupported/broken | Shown but disabled with reason |

The UI must never blur these tiers.

### 4.4 Source adapter interface (proposed)

```rust
trait NovelSource {
    fn profile(&self) -> &SourceProfile;
    async fn search(&self, query: &str) -> Result<Vec<NovelResult>>;
    async fn metadata(&self, url: &Url) -> Result<NovelMetadata>;
    async fn chapter_index(&self, url: &Url) -> Result<NovelChapterIndex>;
    async fn chapter(&self, url: &Url) -> Result<ChapterContent>;
}
```

But prefer config-driven profiles for simple sites and code only for exceptional ones.

### 4.5 Source priority

Start with sources that are:

1. English,
2. searchable,
3. carry large catalogues,
4. not known to be dead,
5. structurally similar enough to reuse a profile.

Do not start by adding all 338. Add 3–5, measure, then scale.

### 4.6 Source inventory commands

Use these to refresh the snapshot:

```bash
LNCRAWL=~/.local/share/novelforge/tools/lncrawl-venv/bin/lncrawl
$LNCRAWL sources list -o json > /tmp/lncrawl-sources.json
$LNCRAWL sources list -s true -o json > /tmp/lncrawl-searchable.json
$LNCRAWL sources list --with-rejected -o json > /tmp/lncrawl-all.json
```

Current counts (2026-11-15):

- 541 total records,
- 339 enabled,
- 202 disabled,
- 49 enabled searchable,
- 21 enabled manga,
- 0 enabled `ko` language sources.

### 4.7 Enabled searchable sources snapshot

This is the current 49-source list. Verify before relying on it; the catalogue changes.

```text
fr    https://novelfrance.fr/
fr    https://lightnovelfr.com/
id    https://www.webnovelover.com/
id    https://wbnovel.com/
id    https://fuyu-novel.my.id/
ru    https://wuxiaworld.ru/
pt    https://tsundoku.com.br/
pt    https://illusia.com.br/
pt    https://phoenixnovels.com.br/
tr    https://turkcenoveloku.blogspot.com/
tr    https://lnmakineceviri.blogspot.com/
es    https://www.skynovels.net/
es    https://novelaenespanol.com/
en    https://gravitytales.com/
en    https://kaoritranslation.blogspot.com/
en    https://eatapplepies.com/
en    https://dobytranslations.com/
en    https://wondernovels.com/
en    https://woopread.com/
en    https://hyacinthbloom.com/
en    https://f-w-o.com/
en    https://battwo.com/
en    https://mto.to/
en    https://dto.to/
en    https://wto.to/
en    https://batotwo.com/
en    https://hto.to/
en    https://readwebnovels.net/
en    https://otakutl.blogspot.com/
en    https://ontimestory.eu/
en    https://www.lightnovelpub.org/
en    https://ladasblmtl.blogspot.com/
en    https://lightnovelheaven.com/
en    https://librarynovel.com/
en    https://zetrotranslation.com/
en    https://vipnovel.com/
en    https://xperimentalhamid.com/
en    https://cangji.net/
en    https://ckandawrites.online/
en    https://novelbuddy.me/
en    https://novelbuddy.io/
en    https://novelbuddy.com/
en    https://home.novel-gate.com/
en    https://novelfire.net/
en    https://noicetranslations.blogspot.com/
en    https://novellatte.blogspot.com/
en    https://novelphoenix.net/
en    https://novelhard.com/
en    https://sammyandpassion.com/
```

---

## NF-023 — NovelFull metadata title suffix and missing author

**Status:** `[x] DONE` (2026-11-15)  
**Priority:** P2  
**Area:** `src-tauri/src/metadata.rs`, `src-tauri/src/sources.rs` (`ParserKind`), `src-tauri/src/export.rs`, external lncrawl `sources/en/n/novelfull.py`

### Evidence

- Live NovelFull export (NF-019, `/tmp/opencode/nf019-qa.log`) wrote
  `title = "Reverend Insanity | Novelfull"` and `author = null`.
- The page's `og:title` includes the site suffix while `h1.tit` is the clean
  title.
- The author is present in the markup as `.m-info .item a[href="/author/…"]`
  (`Gu Zhen Ren`, `蛊真人`), but the generic parser only checks author meta tags,
  JSON-LD and `[itemprop="author"]`.
- The lncrawl `NovelFullTemplate` uses `h3.title` and `.info a[…]`, which no
  longer match, so the EPUB's `dc:title` also carried the suffix and
  `dc:creator` was empty.

### Why it matters

NovelFull is now `verified`; its EPUB and chapter pipeline are correct, but the
library card showed a site suffix and no author. FreeWebNovel and NovelFire
already produce clean metadata, so this was the remaining quality asymmetry.

### Fix plan

1. Add `ParserKind::NovelFull` (or a generic fallback ordered after explicit
   meta tags) that reads `h1.tit` / `h3.title` for the title, the author links
   on the novel page, `.desc-text` for the synopsis and
   `#list-chapter[data-total-chapters]` for the count.
2. Keep `og:title` as the fallback so sources without a visible heading are
   unaffected.
3. Add unit fixtures for the live NovelFull markup and a regression test for
   the current FreeWebNovel/NovelPhoenix titles.
4. Re-run the NF-019 QA download and confirm the clean title and author in
   `metadata.json` and the EPUB metadata.

### Acceptance criteria

- [x] NovelFull `metadata.json` title has no `| Novelfull` suffix.
- [x] `author` is `Gu Zhen Ren` and `authors` contains both live entries.
- [x] FreeWebNovel and NovelPhoenix metadata tests stay green.
- [x] A live NovelFull 1–3 download still audits 3/3 and exports a verified EPUB.

### Resolution

Complete (2026-11-15).

**App-level parser.** `sources.rs` adds `ParserKind::NovelFull` and uses it for
`novelfull.net`. `metadata.rs::parse_novelfull_metadata` now reads:

- title from `h1.tit`, falling back to `og:title` with a trailing
  `| NovelFull` / `- NovelFull` removed by `strip_site_suffix`;
- every credited name from `a[href^="/author/"]` (both `Gu Zhen Ren` and
  `蛊真人`), with `author` set to the first;
- tags from `a[href^="/genre/"]`, status from `/latest-release-novel`,
  synopsis from `.m-desc > div.txt`, and the count from
  `#list-chapter[data-total-chapters]`.

The generic parser is unchanged, so other sources keep their existing
behaviour. `metadata_cover_url`, `index_cache_key` and `fetch_chapter_index`
route NovelFull through the generic index walker.

**lncrawl adapter.** The external `NovelFullTemplate` assumed `h3.title` and
`.info` author links. A second repository-owned patch,
`scripts/patches/lncrawl-v4.14.0-novelfull-metadata.patch`, sets:

```python
novel_title_selector = "h1.tit, h3.title"
novel_author_selector = (
    ".m-info .item a[href*='/author/'], "
    ".info .item a[href*='/author/'], "
    ".info a[href*='/a/'], "
    ".info a[href*='/au/'], "
    ".info a[href*='author']"
)
```

`scripts/apply-lncrawl-patches.sh` applies it after the NF-019 AJAX patch, and
still compiles every patched adapter.

**Export.** `export_epub` now prefers `metadata.author` (the primary name) over
the EPUB's comma-joined `dc:creator`; `authors` continues to carry every name.

### Evidence

- Direct lncrawl probe (`/tmp/opencode/nf023-lncrawl.log`): EPUB
  `dc:title = "Reverend Insanity"`, `dc:creator = "Gu Zhen Ren, 蛊真人"`.
- End-to-end QA (`/tmp/opencode/nf023-qa3.log`):
  - `Metadata · Reverend Insanity · Gu Zhen Ren · 2334 chapter(s)`;
  - `Chapter index · 2334 chapter(s) … all are listed.` (cache hit);
  - `Chapter audit · 3/3 chapters present and non-empty`;
  - `Structured export · metadata.json · 3 chapter file(s) · cover.jpg`;
  - `Success — 1 verified output file(s)`; total **53.9 s**;
  - destination `/tmp/opencode/nf023-qa3/…/reverend-insanity-f6d05df5`.
- `metadata.json`: `title="Reverend Insanity"`, `author="Gu Zhen Ren"`,
  `authors=["Gu Zhen Ren","蛊真人"]`, `chapterCount=2334`,
  `downloadedChapters=3`, `indexComplete=true`, `complete=true`,
  `verified.ok=true`.
- Output tree: `cover.jpg`, `metadata.json`, three chapter XHTML files,
  `chapters/style.css`, `Reverend Insanity.epub` (61,542 bytes, verified).
- EPUB internals: `dc:title = "Reverend Insanity"`,
  `dc:creator = "Gu Zhen Ren, 蛊真人"`.
- Patch reproducibility: the metadata patch applies on top of NF-019 on a fresh
  lncrawl `v4.14.0`; the applier handles all four patches and a re-run skips
  them. Tests **96/96**; `npm run build` passing.

---

## NF-024 — Free public binary release and donation channel

**Status:** `[ ] OPEN` (proposal; details pending approval)  
**Priority:** P2  
**Area:** packaging, README, release docs, About support link

### Evidence

NovelForge currently ships only as a local development build. There is no
public installer, checksum, changelog, license bundle or donation link, and the
app carries no funding UI after NF-014.

### Direction

Store distribution is not viable: Google Play, the App Store and the Microsoft
Store all treat download tools for copyrighted novels as policy violations, and
a rejection can put the developer account itself at risk. The chosen direction
is free public builds with voluntary Ko-fi support — no paywall, no account, no
license key, no gated feature.

### Refinement

1. Publish Windows/macOS/Linux installers or portable builds from a release page
   with SHA-256 checksums and, where possible, a signature.
2. Include `README`, `CHANGELOG`, `SECURITY` and a third-party license bundle
   (lncrawl, gallery-dl, aget, Tauri and their dependencies). GPL components
   need their source offer/link.
3. Add one unobtrusive “Support development” link to Ko-fi in About and the
   README; nothing in the app may depend on it.
4. Do not call the release “open source” unless the source is actually
   published; “free build” / “community build” is accurate.
5. Keep a mirror or fallback download location in case a release page is
   removed, and record a takedown-response process.
6. Avoid store submissions; document that decision in the README.

### Acceptance criteria

- [ ] A downloadable installer/portable build exists for at least Linux and
      Windows, with checksums.
- [ ] No feature, update or support path is gated behind payment.
- [ ] Third-party licenses and GPL source links are included.
- [ ] About/README show the donation link without nagging.
- [ ] The README explains the store/distribution policy and responsible use.

### Resolution

_Not started._

---

## NF-025 — Frontend interaction tests stop at the Home shelf

**Status:** `[x] DONE` (2026-11-16)  
**Priority:** P2  
**Area:** tests, frontend

### Evidence

NF-015 added `src/pages/HomePage.test.tsx` (3 tests) covering the NF-013 source
filter. The other behaviours named in the NF-015 plan have no component test:

- search hydration fallback when a source returns no rows or fails,
- the incomplete-index warning and its “N of M chapters listed” detail,
- Library actions (retry, update, delete, adopt, open index),
- task recovery after an app restart.

These are currently proven only by live manual runs, so a refactor can break
them without any offline failure.

### Refinement

1. Render `NovelSearchPanel` with a stubbed manager: partial results, all-source
   failure list, and the empty-query state.
2. Render `LibraryPage` with a stubbed `DownloadManager` and assert the
   incomplete-index warning, the migrated badge and that retry is hidden for
   migrated entries.
3. Assert that clicking Retry/Update/Delete calls the manager with the clicked
   folder (`targetFolder`), matching the NF-007 fix.
4. Add one test for persisted-task hydration: a task list from disk renders
   with its real status and no fake progress.

### Acceptance criteria

- [x] `npm test` covers the search failure/partial-result paths.
- [x] `npm test` covers the incomplete-index warning and migrated entry.
- [x] Library actions are asserted against the exact clicked folder.
- [x] No new test relies on live network access.

### Change

1. **Search panel** (`src/components/novels/NovelSearchPanel.test.tsx`, 5
   tests). `@/lib/tauri` is mocked, so catalogue and metadata calls never touch
   the network. The tests render a stubbed `searchNovels` and cover:
   a partial response (one hit plus one failed source, including the reason and
   the metadata-failure fallback text), an all-source failure list, a zero-hit
   response that must not invent rows, the `< 2` character search guard, and the
   direct-URL-only source guard.
2. **Library page** (`src/pages/LibraryPage.test.tsx`, 6 tests). A stubbed
   `DownloadManager` carries two novels and an adoptable archive. The tests
   assert the “N incomplete” summary, the **Migrated** badge, and that a
   migrated card has neither **Retry missing** nor **Update**.
3. **Index dialog.** The incomplete-index warning is opened from the card and
   asserted together with its `2,140 of 2,334 chapters listed` detail; the
   migrated card instead shows `local EPUB index` and the EPUB-spine note with
   no remote-index warning.
4. **Exact folder/path.** Retry, update, delete and adopt are clicked on the
   second card/archive and asserted to call the manager with that card's
   `folder` (or the archive's `path`) exactly once. This locks in the NF-007
   `targetFolder` fix, including the delete confirmation dialog.
5. **Restart hydration** (`src/hooks/useDownloadManager.test.tsx`, 1 test).
   `api.getTasks` returns two persisted tasks (paused at 42%, completed at 100%
   with `Verified`). The hook boots through the real `DownloadsPage`, which
   must render those stored statuses and percentages instead of fabricated
   progress.

### Evidence (2026-11-16)

`npm test`: **6 files, 23 tests passed** (was 3 files, 11 tests). The three new
files hold 12 tests. Mocked `api` methods are asserted through `vi.mocked`, and
the search suite never calls `fetchNovelMetadata`, `fetchNovelChapters`,
`findExistingDownload`, `listSourceCatalogue` or `searchNovels` for real.
`bash scripts/check.sh` passed: Rust **129 unit + 2 integration**, frontend
**23/23**, `npm run build` passing.

---

## NF-026 — Author search misses novels listed on the site author page

**Status:** `[x] DONE` (2026-11-16)  
**Priority:** P1  
**Area:** resolver, source registry, metadata

### Evidence

Searching the author `Gu Zhen Ren` returned only *Reverend Insanity* and
*Mysteries of Immortal Puppet Master*, while the same site listed four novels by
that author. Live checks on 2026-11-16:

| Probe | Result |
| --- | --- |
| `novelphoenix.net/search?keyword=Gu%20Zhen%20Ren` | 2 rows (the same two) |
| `novelphoenix.net/author/Gu+Zhen+Ren` (JSON-LD + cards) | 2 novels |
| `novelfire.net/author/gu-zhen-ren` | **4 novels**: Reverend Insanity, Infinite Bloodcore, Mysteries of Immortal Puppet Master, Advent of Immortal Truth |
| `lncrawl search "Gu Zhen Ren" -s https://novelfire.net/` | 10 unrelated title rows, no author rows |

The resolver only ever looked at title-search rows and then verified candidate
detail pages, so a novel the search endpoint does not return could not be found
at all — even though the site itself publishes the complete list under
`/author/<name>`.

### Change

1. **Registry.** `SourceProfile` now declares `author_page` (a template with
   `{author}`) and `author_slug` (`AuthorSlug::Hyphen` → `gu-zhen-ren`,
   `AuthorSlug::Plus` → `Gu+Zhen+Ren`). NovelFire and NovelPhoenix declare one;
   profiles without an author page declare `None`. Validation rejects a
   template without `{author}` or without `https://`.
2. **Parser.** `metadata::fetch_author_page` reads the page through the shared
   transport and prefers a JSON-LD `ItemList` (NovelPhoenix), then the first
   `ul.novel-list` (NovelFire — later lists are “You may also Like”), then
   `h3.tit`, then titled anchors inside `main`. The author name comes from the
   breadcrumb, the `h1` or the JSON-LD `CollectionPage`; the page's canonical
   URL is captured so a redirect to the home page can be rejected.
3. **Author mode.** Author pages are the primary answer. Only sources without a
   usable page fall back to crawler search + detail verification, so the
   default registry needs no `lncrawl` process at all.
4. **Auto mode.** Author pages run concurrently with the search. An exact
   author-name query (case/diacritic-insensitive) is answered from the pages and
   skips detail verification; otherwise the page rows are merged into the
   alternatives after a search hit matched on author.
5. **Ordering.** The bibliography keeps the shelf order of the first source
   that lists each novel; when several sources carry the same title, the
   download-ready source supplies the URL. Duplicate titles are never listed
   twice.

### Live evidence (2026-11-16)

| Probe | Before | After |
| --- | --- | --- |
| Author `Gu Zhen Ren` (all sources) | 2 novels, 12.0 s | **4 novels**, 1.6–2.1 s |
| Author `Gu Zhen Ren` (NovelPhoenix only) | 2 novels, 7.9 s | 2 novels, 1.5 s |
| Author `Gu Zhen Ren` (NovelFire only) | no match, 13.8 s | 4 novels, 1.5 s |
| Auto `Gu Zhen Ren` | 2 novels, 12.6 s | 4 novels (Reverend Insanity + 3 alternatives), 4.8–5.5 s |
| Auto `reverend insanity` | 1.00 title | unchanged 1.00 title, 4.8 s |
| Author `Nobody McFakename` | no match | no match (falls back to search, 1.8 s; NF-027) |

Desktop run (Author toggle → Resolve source): “100% author match”, *Reverend
Insanity* with “AUTHOR “GU ZHEN REN” ON NOVELPHOENIX.NET AUTHOR PAGE”, and the
three other novels as other-source matches.

### Known limits

- Only sources that declare `author_page` can answer from an author page;
  NovelFull, FreeWebNovel, Divine Dao Library and NovelPing are direct-URL-only
  in the registry and are not searched for authors.
- NovelFire answers an unknown author slug with its **home page** (HTTP 200, 67
  cards). The canonical-path and author-label checks reject it, but a future
  layout change must keep at least one of those signals working.
- Only the first author page is read; neither live source paginates the four
  and two novels observed.
- NovelPhoenix orders its author page alphabetically (JSON-LD
  `ItemListOrderDescending`), so its first novel is not necessarily the most
  popular; NovelFire's “Most Popular” order wins when both list a title.

### Tests

`author_pages_are_declared_with_a_slug_style`, `author_slug_filters_path_characters`,
`author_page_reads_only_the_author_shelf`, `author_page_prefers_json_ld_item_list`,
`author_page_urls_use_the_source_spelling`,
`author_page_hits_keep_shelf_order_and_prefer_ready_sources`,
`author_page_resolution_lists_the_whole_shelf_once`. Rust **129 unit + 2
integration**, frontend 11/11, `npm run build` passing.

---

## NF-027 — Search/resolve latency

**Status:** `[x] DONE`  
**Priority:** P1  
**Area:** resolver, crawler invocation, caching

### Evidence (measured 2026-11-16, default sources NovelFire + NovelPhoenix)

Before NF-027:

| Probe | Time |
| --- | --- |
| `lncrawl search "Gu Zhen Ren" -s https://novelphoenix.net/` | 5.8 s |
| Author resolve, all sources (before NF-026) | 12.0 s |
| Author resolve, NovelFire only (before NF-026) | 13.8 s (no match) |
| Author resolve, NovelPhoenix only (before NF-026) | 7.9 s |
| Auto `Gu Zhen Ren` (after NF-026) | 4.8–5.5 s |
| Auto `reverend insanity` | 4.8 s |
| Auto `master of gu` (AniList + Wikipedia) | 17.1 s |
| Author `Nobody McFakename` | 15.6 s |

After NF-027 (debug build, cache dir `/tmp/opencode/nf027-cache`):

| Probe | Cold | Warm |
| --- | --- | --- |
| Author `Gu Zhen Ren` (two sources, author mode) | 1.72 s | **0.00 s** (`cached=true`, age 5 s) |
| Title `reverend insanity` (auto) | 5.17 s | **0.74 s** (`cached=true`, age 8 s) |
| Author `Nobody McFakename` (no match) | 1.82 s | 0.84 s |
| Search `Gu Zhen Ren` (catalogue batch) | 5.2 s | **0.0 s** (`cached=true`, age 4 s) |
| Auto `master of gu` (AniList synonym) | 14.27 s | **1.68 s** (`cached=true`, age 243 s) |

Logs: `/tmp/opencode/nf027-author-cold.log`, `nf027-author-warm.log`,
`nf027-title-cold.log`, `nf027-title-warm.log`, `nf027-author-miss-cold.log`,
`nf027-author-miss-warm.log`, `nf027-search-warm.log`, `nf027-search-warm2.log`,
`nf027-mastergu-cold.log`, `nf027-mastergu-warm.log`.

A cold resolve still pays one `lncrawl` Python start (roughly 5 s) because the
crawler process itself is not reusable; a warm resolve avoids the process
entirely and answers from disk.

### Causes

1. Every source search spawns `lncrawl` (Python) afresh: ~1.5–2 s startup
   before any HTTP request, 20 s per-source timeout, 40 s command budget.
2. Author/title verification fetches up to 8 detail pages at 3 concurrency with
   a 12 s timeout each; junk search rows (NovelFire author queries) make this
   the dominant cost.
3. When source matching fails, AniList and Wikipedia are consulted and the
   sources are searched **again** with the canonical title.
4. Search rows and detail summaries are not cached between the resolve call and
   the picker, so opening a mirror repeats the same requests.

### Implemented

1. `cache.rs` adds a generic on-disk TTL cache. Each entry stores its own key,
   so a hash collision cannot return another query's rows. Successful search
   batches, author pages and detail summaries are cached for 15 minutes
   (`NOVELFORGE_SEARCH_TTL_SECS`), with `NOVELFORGE_SEARCH_CACHE_DIR` for an
   explicit cache location.
2. `search_targets` reads a per-source batch from disk before spawning the
   crawler, and reports `cached` plus the oldest `cacheAgeSeconds` on
   `SearchResponse`.
3. `verify_candidates` verifies candidates in waves. The first wave is three
   pages; an exact author score (`>= 0.95`) ends verification immediately.
   Later waves use concurrency 6 with an 8 s per-summary timeout, and every
   summary is cached.
4. `collect_author_pages` caches author pages and treats an empty page as
   authoritative when both the canonical path contains `/author` and the stated
   author label matches the query, so no fallback title search runs.
5. A second crawler search is skipped when the canonical title equals the input
   or already appears in the first result set.
6. The QA harness prints `cached`/`cache_age` and elapsed time for `search` and
   `resolve`, and failures now print their elapsed time.
7. The UI labels cached data: the results header shows `cached <age>` beside the
   query, and the resolved panel appends `· cached <age>` to its searched-source
   line.

### Not pursued

- **One crawler invocation for several sources.** The installed lncrawl CLI
  accepts a single source per `-s`: repeating `-s` keeps only the last source,
  a comma-separated list returns “No searchable sources available”, and a
  space-separated list is treated as one FTS phrase. Multi-source invocation is
  therefore an upstream CLI limitation, not a NovelForge bug. The per-source
  disk cache removes the cost on repeat resolves, which is what this item was
  meant to protect.

### Acceptance criteria

- [x] A two-source author resolve stays under 1.5 s and a title resolve under
      3 s on a warm cache (author 0.00 s, title 0.74 s).
- [x] Candidate verification never fetches more than the first three candidates
      once an exact author match is known
      (`exact_author_in_the_first_wave_stops_after_three_fetches`).
- [x] Cached responses are clearly labelled (`cached`/`cacheAgeSeconds`) wherever
      the result is shown (search header, resolved panel, QA output).
- [x] Live probes in this table are re-measured and recorded above.

---

## NF-028 — Book search middle layer and explicit candidate selection

**Status:** `[x] DONE` (2026-11-16)  
**Priority:** P0  
**Area:** books resolver, `aget`, Books UI

### Evidence (2026-11-16)

The current books path hands the raw query to `aget` and accepts whatever the
first EPUB-looking row is:

- `scripts/aget.py::welib_search` returns the **first** `.book-card` that
  mentions EPUB and has an `auto_download` link.
- `scripts/aget.py::annas_download` returns the **first** row whose text
  contains the format string.
- `scripts/aget.py::ocean_download` sorts result URLs by a loose token score
  and takes `links[0]`.

There is no title, author, year or ISBN comparison. A search for `fourth wing`
can therefore download a different book that happens to appear first.

Live probes:

| Probe | Result |
| --- | --- |
| `aget "fourth wing" --source welib --auto` | `https://welib.st: HTTP Error 403: Forbidden`, no result in under 1 s |
| `aget "fourth wing" --source annas --auto --headless` | still printing `searching…` when killed at 95 s (DDoS-Guard wait loop is 16 × 4 s, then the slow page polls 14 × 4 s) |
| `aget "fourth wing" --source ocean` | code launches `nodriver` with `headless=False` and `--window-position=-2000,-2000`; on Hyprland the window was visible |
| User report | “wrote fourth wing and it downloaded whatever book came first” |

The search costs are structural, not transient: Anna's and Ocean are behind
bot challenges, and `aget` is a scraper with no metadata contract.

### Proposed design

1. **Metadata first.** Add a `search_books` command that queries a real book
   catalogue and returns candidates with `title`, `authors`, `year`, `isbn`,
   `publisher`, `language`, `series`, `seriesPosition`, `coverUrl`, `edition`
   and a stable catalogue id. Primary: Hardcover GraphQL
   (`search(query, query_type: "Book")`); fallback: Open Library
   `search.json` (`title`, `author_name`, `first_publish_year`, `isbn`).
2. **Show candidates, never auto-pick.** The Books page renders a result list
   (cover, title, author, year, series position, format). The user selects the
   exact row. A **Fetch** button sends the selected catalogue record to the
   backend, not just a string.
3. **Rank file records against that metadata.** When a source returns file
   rows, score title, author, ISBN, language and format; require a threshold
   before download. If no row clears it, show the near misses and let the user
   choose instead of downloading the first one.
4. **Background only.** No headed Chromium in the default path. OceanOfPDF
   becomes an explicit manual fallback (or is removed; see NF-030).
5. **Source availability first.** The result row shows which sources can serve
   the book before the user commits, and each source reports its own failure
   (403, challenge, no EPUB) instead of silently falling through.

### Implementation (2026-11-16)

- **Catalogue middle layer.** `src-tauri/src/books.rs` adds `search_books`
  with Hardcover GraphQL (local token only) and an Open Library fallback.
  Rows carry `title`, `authors`, `year`, `isbn`, `publisher`, `language`,
  `series`, `seriesPosition`, `coverUrl`, `source` and a stable id, cached for
  one hour under `book-cache` (`books::v7`).
- **Series resolution.** Open Library runs the title search and
  `series:"<name>"` together, then checks every candidate work's
  `editions.json` for real membership and a position (`#1`, `#3.5`,
  `, Book 2`, `-- 2`). Fuzzy title hits, omnibus rows and companion books are
  dropped, and a position is never invented. Lookups run five at a time with
  429/5xx retry (700 ms/1400 ms) so a throttle cannot silently produce an
  empty series.
- **Verified file selection.** `scripts/aget.py` gains a `libgen` source
  (`LibgenRowParser`) and scores every row with `title_similarity`,
  `author_similarity`, `language_rank` and an ISBN bonus. `choose_candidate`
  requires `MATCH_THRESHOLD = 0.62` and **fails closed** — it never falls back
  to `rows[0]`. Anna's waits are bounded; Ocean is headless by default and
  verifies the title per candidate.
- **Rust wiring.** `DownloadOptions` carries `bookTitle`/`bookAuthor`/
  `bookIsbn`; `process.rs` validates the ISBN and passes `--title/--author/
  --isbn`; `search_books`, `get_hardcover_status`, `set_hardcover_token` and
  `clear_hardcover_token` are registered in `lib.rs`.
- **Books UI.** Catalogue result list, source cards, series checklist with a
  per-book **Fetch** and queue action, and the existing shelf. Nothing is
  downloaded until the user selects rows. `SettingsPage` stores the optional
  Hardcover token in the local app-config folder (mode `0600`); the token is
  never returned by `get_settings`.
- **QA hooks.** `NOVELFORGE_QA_BOOKS`/`NOVELFORGE_QA_BOOKS_MODE` drive
  catalogue/series runs; `NOVELFORGE_QA_BOOK_TITLE`/`_AUTHOR`/`_ISBN`/`_SOURCE`
  drive a real download through the app path.

### Live evidence (2026-11-16)

| Probe | Result |
| --- | --- |
| `NOVELFORGE_QA_BOOKS="The Empyrean"` (cold) | series *The Empyrean*, Rebecca Yarros, 3 books: *Fourth Wing* #1, *Iron Flame* #2, *Onyx Storm* #3; 8.07 s (`/tmp/opencode/qa-empyrean9.json`) |
| `NOVELFORGE_QA_BOOKS="The Lord of the Rings"` (cold) | series *The Lord of the Rings*, Tolkien, 4 books: *Fellowship* #1, *Two Towers* #2, *Return of the King* #3, *The Hobbit* unpositioned; 14.93 s (`/tmp/opencode/qa-lotr6.json`) |
| `NOVELFORGE_QA_BOOKS="Dune"` | 20 candidates, series *Dune*, expected titles first |
| App-path download: *Fourth Wing*, Rebecca Yarros, ISBN `9781649374042`, LibGen | `Fourth Wing.epub` 5,064,906 bytes plus `metadata.json`, `index.json`, `chapters.html`, `cover.jpg`, 46 chapter XHTML, `Images/` and `Styles/` (`/tmp/opencode/qa-book-download.log`) |
| Fail-closed probe: expected *The Hobbit* / Tolkien | no file written; near misses reported instead of downloading the first row |
| Gates | `npx tsc --noEmit` clean, `npm test` 27/27, `cargo test --lib` 148/148, `bash scripts/check.sh` green |

### Acceptance criteria

- [x] `fourth wing` returns *Fourth Wing* by Rebecca Yarros as the top
      candidate, with year and series position, and no download starts until a
      candidate is selected. Open Library rows without an edition ISBN fall
      back to year/source as identity, which the UI shows.
- [x] A decoy first result is not downloaded and a missing title/author match
      fails closed (live probe above). The permanent fixture form is NF-031.
- [x] Default fetch never opens a visible browser window: the LibGen path is
      pure HTTP and Ocean only runs when explicitly selected.
- [x] A 403/challenged source is reported as a per-source failure instead of
      hanging; warm-cache catalogue search returns immediately and a cold
      series search lands at 8–15 s.
- [x] The selected record is what lands in the library: title/author are
      scored before download and the EPUB is audited afterwards. ISBN is a
      scoring bonus, not a check inside the EPUB file.

Series layout, skip-already-complete and per-book `not found` reporting remain
in NF-029; source measurement, Ocean headless verification and the shipped
default order remain in NF-030.

---

## NF-029 — Series automation

**Status:** `[x] DONE` (2026-11-16) — series resolution, one-action queue, the
`Author/Series/NN - Title/` layout, skip-complete and per-book `not found`
reporting are all live. Evidence below.  
**Priority:** P1  
**Area:** books resolver, job queue, Books UI, library naming

### Goal

Entering a series name (for example `The Empyrean`) or any book in it should
resolve the whole series, show the books in order, and download every released
EPUB in one action.

### Proposed design

1. **Series resolution.**
   - Hardcover: `search(query, query_type: "Series")` →
     `series.book_series(distinct_on: position, order_by: [{position: asc},
     {book: {users_count: desc}}])` with the documented filters for merged
     books, partial books and compilations.
   - Fallback: Open Library `q=series:"<name>"` (live probe returned *Fourth
     Wing*, *Iron Flame* and *Onyx Storm* for `The Empyrean`).
   - Last resort: Goodreads book page → `/series/<id>` page (HTML; fragile).
2. **Series card.** Cover, author, ordered list with positions, release dates,
   `released`/`upcoming` status, and which books already exist in the library.
   The user can deselect books; released books are selected by default.
3. **Per-book pipeline.** For each selected book: metadata candidate → source
   candidates → explicit/verified fetch (NF-028) → EPUB audit → library import.
4. **Naming and layout.**
   `Books/<Author>/<Series>/<NN> - <Title>.epub`, with `metadata.json`,
   `index.json` and the existing verification sidecars. Position comes from the
   catalogue, not from the download order.
5. **Partial success is normal.** A missing book stays visible as
   `not found` with the sources tried; the rest of the series still completes.
   Unreleased books are skipped, not failed.

### Acceptance criteria

- [x] `The Empyrean` resolves to *Fourth Wing*, *Iron Flame*, *Onyx Storm* (and
      any later released book the catalogue lists) in position order. Live
      probe: `/tmp/opencode/qa-empyrean9.json`; *The Lord of the Rings* resolves
      1/2/3 plus *The Hobbit* in `/tmp/opencode/qa-lotr6.json`.
- [x] A single action queues all released books and writes them under
      `Author/Series/NN - Title.epub` with correct metadata. The series card
      queues every selected released book in one action (`queueSeries`), and
      each job shelves into `<download root>/<Author>/<Series>/<NN> - <Title>/`
      with `<NN> - <Title>.epub`, `metadata.json`, `index.json`,
      `chapters.html`, cover and chapter folders. Live: *Fourth Wing* →
      `…/Rebecca Yarros/The Empyrean/01 - Fourth Wing/01 - Fourth Wing.epub`
      (5,064,906 bytes) plus sidecars and 46 chapters
      (`/tmp/opencode/qa-029-download2.log`). Unpositioned books omit the
      `NN - ` prefix. Rust: `series_layout_uses_author_series_and_padded_position`,
      `series_layout_falls_back_without_a_position_and_stays_contained`.
- [x] Compilations, partial books and duplicate positions are filtered out:
      title-matching omnibus rows and `collection`/`box set`/`omnibus` titles
      are dropped, and positions are read from editions rather than inferred.
- [x] An unpublished book is marked `upcoming` and does not fail the run
      (release-date handling lives in `books.rs`).
- [x] A missing book reports `not found` without blocking the other downloads.
      A Books failure with no EPUB now ends with `Book not found: no
      catalogue-verified EPUB was produced (network failure, mirror block, or
      no row cleared the match threshold)…` instead of a bare transport error
      (`/tmp/opencode/qa-029-download2.log`). Each book is its own job, so the
      rest of the series keeps running.
- [x] Re-running the series skips books already complete and does not create
      duplicate folders. The UI marks a complete title + compatible author as
      `already shelved`, excludes it from `queueSeries`, and the backend refuses
      an existing complete series folder: `start failed: “Fourth Wing” is
      already complete in the library at …/01 - Fourth Wing`
      (`/tmp/opencode/qa-029-skip3.log`). `list_library` still sees the nested
      novel as `complete: true` (17,738,903 disk bytes,
      `/tmp/opencode/qa-029-library.log`). Tests:
      `BooksPage.test.tsx` “skips series books that are already complete in the
      library”.

---

## NF-030 — Book source evaluation and default policy

**Status:** `[x] DONE` (2026-11-16) — the four free sources are measured end to
end, the default order is justified by that evidence, and every failure is
bounded and visible.  
**Priority:** P1  
**Area:** books engines, settings, Guardrails

### Current state (2026-11-16)

`scripts/aget.py` exposes `libgen` (pure HTTP, HTML row parser), `welib`,
`annas` and `ocean`. The default order is `libgen → welib → annas → ocean`;
Ocean only runs when explicitly selected, and then headless.

NF-030 changes:

- Every source is bounded by one **per-source budget** (default 150 s,
  `--budget` / `NOVELFORGE_SOURCE_BUDGET`). Polling loops check the deadline, so
  a DDoS-Guard/Cloudflare hold ends in one visible
  `… after N.Ns — skipped` line instead of a hung job. Anna's download wait was
  previously 10 × 180 s; it is now capped by the same budget.
- LibGen search adds the expected author (`Pride and Prejudice Jane Austen`) and
  requests 100 rows, because the mirror sorts by recency and a bare classic
  title surfaced only variations. Bare-title search remains the fallback.
- WeLib checks both mirrors and reports `HTTP 403` / “no EPUB cards” in seconds.
- Anna's rows no longer require author metadata that the site does not expose:
  an exact title renormalises over the available signals (0.92 for *Dune*),
  while a fuzzy title without an author still fails.
- Ocean headings are cleaned (`[PDF] [EPUB] … Download` → the title) before the
  0.62 title check, and a stale Chromium profile lock is cleared before launch
  (an orphaned headless Chromium used to turn every Ocean run into
  `Failed to connect to browser` in 3 s).
- **Selection guards found by the measurements themselves:**
  - an omnibus/collection (`… Collection`, `… Omnibus`, `Complete Works`,
    `Books 1 - 6`) is rejected unless the user asked for one;
  - the title must reach 0.55 on its own, so `Dune Messiah` no longer rides on
    an exact author match (it scored 0.72 before this floor);
  - a punctuation suffix (`(The Empyrean, #1)`, `: An Invention`) needs the
    author to confirm it, which is what separates a subtitle from a derivative
    work such as `Dune: Tales of Dune Expanded Edition`;
  - the author score is divided by the *row's* names, so
    `Frank Herbert; Brian Herbert & Kevin J Anderson` no longer scores 1.0;
  - a row that declares a non-English language is rejected when no `--lang` was
    requested (`Dune 2a ed`, Spanish, scored 0.65 and was downloaded before this
    rule);
  - Anna's title-only renormalisation still requires a near-exact title
    (≥ 0.9), so a fuzzy title without author metadata fails closed.
- LibGen keeps 25 s of its budget for the transfer, caps each mirror search at
  15 s, and downloads from up to three verified copies, because one storage node
  stalling used to consume the whole budget for a book that was available.
- `scripts/measure_sources.py` runs the same five titles against every source
  and writes JSON + Markdown evidence to `docs/measurements/`; timeout handling
  kills the whole process group (Playwright/Chromium grandchildren included),
  each row starts from an empty directory, and `--merge` re-renders the report
  from saved results.

**User decision (2026-11-16):** keep `libgen → welib → annas → ocean`; NF-024
stays parked. This is an explicit product decision that overrides the
“shadow libraries must not be the default” note in the licensing boundary.
Lawful-source support is split into NF-032.

### Candidate sources

| Source | Metadata | File access | Notes |
| --- | --- | --- | --- |
| Hardcover | GraphQL, series-aware, free token (5,000 req/day) | none | Best metadata/series authority; needs a user token and `read:catalog` scope |
| Open Library | `search.json`, no token | none (lending only) | Good fallback; series field is sparse but `series:"..."` search works |
| Anna's Archive | HTML/JSON; search needs membership tier | Fast-download API with member key; free path is slow and challenged | Measured 2026-11-16: 4/5 verified EPUBs, 38.6–80.0 s; one DDoS-Guard hold hit the 150 s budget. Never paid. |
| LibGen | HTML/JSON mirrors | Direct `.epub` mirrors | Measured 2026-11-16: 3/5 verified EPUBs, 9.4–17.6 s, pure HTTP. One title matched but its storage node stalled; another (*Dune*) has no single-volume English EPUB on the first page. |
| WeLib | HTML | Direct/auto-download | Measured 2026-11-16: `.gl` served a CAPTCHA and `.st` returned HTTP 403; both skipped inside 3 s. |
| OceanOfPDF | HTML | Cloudflare Turnstile + headless Chromium | Measured 2026-11-16: headless and `--headless=new` never cleared Turnstile (150 s / 60 s); explicit-only, bounded by the source budget |
| Shelfmark / stacks | optional external service | multiple | Good reference implementations; too heavy to bundle in a desktop app |

### Review note (2026-11-16) — two suggested repos

| Repo | Stack | Verdict |
| --- | --- | --- |
| `Krishna-Sivakumar/opdf-scraper` | `requests` + BeautifulSoup, last commit 2025-06 | No challenge handling at all. Scrapes `recently-added` listing pages and `Fetching_Resource.php` form actions. Dies on the current `403 Just a moment...` |
| `maddisanotoole/bulk-epub-downloader` | FastAPI + queue worker, `cloudscraper` 1.2.71, 2026 | Live probe returned `403 Just a moment...` with the default scraper, `interpreter=nodejs`, and a native-browser fingerprint block |

Neither removes the browser requirement. `cloudscraper` only clears the legacy
I'm Under Attack Mode challenge; OceanOfPDF now serves Cloudflare's managed
challenge/Turnstile. Every working solver (`nodriver`, Playwright,
undetected-chromedriver, FlareSolverr) runs the challenge in a real browser
engine and must reuse a `cf_clearance` cookie bound to the same IP and UA. There
is no pure-HTTP path today, and maintaining one would be a permanent
fingerprint arms race. Anna's (DDoS-Guard) and WeLib (`/verify_captcha`) behave
the same way.

### Licensing boundary

LibGen, Anna's Archive, WeLib and OceanOfPDF serve in-copyright commercial
titles. Automating series downloads of those titles is copyright infringement,
so the default path must not do it and NovelForge must not ship a bulk fetcher
for them. In scope instead:

1. Metadata, series resolution and library organisation for any book the user
   already owns.
2. Automated download from lawful free catalogues: Project Gutenberg (HTML
   search plus `ebooks/<id>.epub3.images`, verified `200 application/epub+zip`)
   and Standard Ebooks (`/feeds/atom/new-releases`, verified
   `200 application/atom+xml`).
3. EPUB import/verify from a local file or a URL the user supplies, with
   `Author/Series/NN - Title.epub` layout, ISBN/hash dedupe and the EPUB audit.
4. OceanOfPDF: remove the automated engine. A plain "open in browser" link is
   acceptable; no hidden Chromium.

A source is only eligible for the default path if it distributes the file
lawfully.

> **Superseded for the default order (2026-11-16):** the user explicitly chose
> to keep `libgen → welib → annas → ocean` and parked NF-024. That decision is
> recorded above and is not a silent doc edit. The boundary still applies to any
> bundled lawful-source work, which is tracked separately as NF-032.

### Chosen policy (2026-11-16)

1. Metadata: catalogue-first. Hardcover is used when a user token is stored
   locally; Open Library is the no-token fallback. Neither sends the token
   anywhere else (NF-028).
2. Files: the user-approved order is `libgen → welib → annas → ocean`. Ocean is
   explicit-only and headless; Auto never starts a browser. Lawful public-domain
   sources are a separate follow-up (NF-032) and do not change this order.
3. Search/detail caching reuses the books TTL cache (NF-027).
4. Per-source success, latency and failure reason are recorded by
   `scripts/measure_sources.py`; the Books source cards carry a dated summary,
   and Guardrails shows aget/Chromium readiness (Chromium is optional now:
   LibGen and WeLib work without it).
5. Every source goes through the same `aget` scoring path; adding a mirror means
   one adapter function and one `SOURCES` entry.

### Acceptance criteria

- [x] Every source is measured on the same five titles with a repeatable
      harness; “success” means the run exited 0 *and* wrote a `PK` EPUB.
      Evidence: `docs/measurements/nf-030-2026-11-16.json` / `.md`.
- [x] The default order is justified by that evidence: LibGen returned a
      verified EPUB for 3 of 5 titles in 9.4–17.6 s; a fourth (*Pride and
      Prejudice*) matched but its file host stalled, and the job fell through to
      Anna's Archive; its one metadata miss was *Dune*, where the mirror has no
      single-volume English EPUB on the first page. Anna's covered *Dune* and 3
      others in 38.6–80.0 s; WeLib was blocked on both mirrors; Ocean never
      cleared Turnstile headlessly.
- [x] A source that is challenged/403 is skipped quickly and visibly. WeLib
      fails in 2.2–2.6 s with `HTTP 403`/`CAPTCHA`; Anna's and Ocean stop at the
      per-source budget with one `… — skipped` line instead of a hung job.
- [x] No default path opens a browser window: LibGen/WeLib are HTTP-only, Auto
      never starts a browser, and Ocean is explicit-only and headless.
- [x] Ocean cannot be made invisible *and* working in this environment, so it
      ships as an explicit choice rather than a silent default: headless stayed
      on Turnstile for 150 s and `--headless=new` for 60 s; headful offscreen
      cleared it in 24 s but is not guaranteed invisible, and
      `NOVELFORGE_OCEAN_HEADFUL=1` is debug-only.
- [x] Selection never blindly takes the first row. The decoys the measurements
      surfaced all score below the 0.62 threshold: omnibus
      `Dune: The Gateway Collection`, anthology
      `Dune: Tales of Dune Expanded Edition`, Spanish `Dune 2a ed`, Dutch
      `Dune - Frank Herbert`, and `Dune Messiah`.
- [x] Shadow-library-by-default is an explicit product decision (2026-11-16),
      recorded above instead of a silent doc edit; lawful public-domain sources
      are tracked as NF-032.

### Measured 2026-11-16 (per-source budget 150 s)

Full artefact: `docs/measurements/nf-030-2026-11-16.{md,json}` — five catalogue
titles × four sources, every row a real `aget` run that had to write a `PK`
EPUB to count as a success. A row that failed for transport reasons
(`URLError`, socket timeout) was retried once; the table shows the retry.

| Title | Source | Result | Seconds | Size | Note |
| --- | --- | --- | ---: | ---: | --- |
| Pride and Prejudice | libgen | fail | 66.6 | — | file host stalled (TimeoutError) after the match |
| Pride and Prejudice | welib | fail | 3.5 | — | `welib.gl` CAPTCHA; `welib.st` HTTP 403 |
| Pride and Prejudice | annas | success | 79.7 | 0.52 MB | best match 0.92 |
| Pride and Prejudice | ocean | fail | 148.7 | — | Turnstile challenge, bounded by the budget |
| The Great Gatsby | libgen | success | 9.7 | 0.70 MB | best match 0.96 |
| The Great Gatsby | welib | fail | 4.1 | — | CAPTCHA / 403 |
| The Great Gatsby | annas | success | 80.0 | 0.30 MB | best match 0.92 |
| The Great Gatsby | ocean | fail | 150.6 | — | Turnstile challenge |
| The Time Machine | libgen | success | 9.4 | 0.48 MB | best match 1.00 |
| The Time Machine | welib | fail | 25.6 | — | CAPTCHA / 403 |
| The Time Machine | annas | fail | 150.4 | — | DDoS-Guard challenge |
| The Time Machine | ocean | fail | 150.7 | — | Turnstile challenge |
| Dune | libgen | fail | 6.9 | — | no row cleared 0.62 (only collections and non-English editions) |
| Dune | welib | fail | 3.8 | — | CAPTCHA / 403 |
| Dune | annas | success | 38.6 | 0.71 MB | best match 0.92 |
| Dune | ocean | fail | 150.7 | — | Turnstile challenge |
| Fourth Wing | libgen | success | 17.6 | 4.83 MB | best match 1.00 |
| Fourth Wing | welib | fail | 2.9 | — | CAPTCHA / 403 |
| Fourth Wing | annas | success | 45.0 | 2.09 MB | best match 0.92 |
| Fourth Wing | ocean | fail | 150.7 | — | Turnstile challenge |

| Source | Success | Median seconds | Failure reasons |
| --- | ---: | ---: | --- |
| libgen | 3/5 | 9.7 | file host stalled; no row cleared 0.62 (*Dune*) |
| welib | 0/5 | 3.8 | both mirrors blocked (CAPTCHA / HTTP 403) |
| annas | 4/5 | 62.4 | one DDoS-Guard challenge |
| ocean | 0/5 | 150.7 | Turnstile never cleared headlessly |

What this buys the product:

- **Auto still delivers.** On the *Pride and Prejudice* row LibGen's storage
  node stalled and WeLib was blocked; the same job would still produce the EPUB
  from Anna's Archive. Fast-fail is what makes that cheap.
- **The two fast sources are the right first two.** LibGen is HTTP-only and
  answered in 9–18 s; WeLib fails in 3 s instead of hanging.
- **Anna's is the reliable fallback**, at the cost of 38–80 s (plus one
  challenge when the site is under load).
- **Ocean stays explicit-only.** It never cleared Turnstile headlessly in this
  environment, so it can only report an honest challenge.
- **Wrong books are now rejected, not downloaded.** *Dune* is the proof: LibGen's
  first page offers the Gateway Collection, a Saga Collection, Brian
  Herbert/Anderson sequels and Spanish/Dutch editions; nothing cleared the
  threshold, so the app failed closed there and got the real novel from Anna's.

---

## NF-031 — Permanent regression fixture for book metadata scoring

**Status:** `[x] DONE` (2026-11-16) — `scripts/test_aget.py` runs 31 offline
tests over two real LibGen search pages plus the NF-032 Gutenberg/Standard
Ebooks pages, and `scripts/check.sh` fails if the selector regresses.  
**Priority:** P2  
**Area:** `scripts/aget.py`, tests

### Why

NF-028 proved the selection logic live (12 LibGen rows → *Fourth Wing* score
1.20; a wrong expectation wrote no file and reported near misses), but there was
no committed fixture that ran `choose_candidate` offline. A future scoring tweak
could have regressed to first-row selection without any gate failing.

### What was added (2026-11-16)

- `scripts/fixtures/libgen/fourth-wing.html` and `dune.html` are **real** search
  pages saved on 2026-11-16 (11 and 13 EPUB rows). `scripts/fixtures/libgen/README.md`
  records the queries, counts and the rule that fixtures are refreshed only from
  a live page, never edited to make a test pass.
- `scripts/test_aget.py` (stdlib `unittest`, no network) covers:
  - parser: the saved pages yield the expected rows with md5/format/author/
    language, and an unrelated page yields nothing;
  - positive selection: the exact English *Fourth Wing* row wins with score 1.00;
  - decoy-only page: `Dune` returns `None` and writes no file (the temp output
    directory stays empty);
  - `choose_candidates`: only rows at or above 0.62, sorted best first, `limit`
    respected, no foreign-language row included;
  - guards: title floor (`Dune Messiah`), omnibus/collection, punctuation suffix
    with and without author confirmation (Gatsby passes, *Tales of Dune* fails),
    co-author author score below the 0.8 floor, declared non-English rejection,
    and an explicit `--lang` still selecting that language;
  - Anna's title-only renormalisation: exact title without author passes, fuzzy
    title without author fails closed;
  - Ocean heading cleanup: `[PDF] [EPUB] Fourth Wing Download` → `Fourth Wing`;
  - a contract test pins `MATCH_THRESHOLD 0.62`, `MIN_TITLE_SCORE 0.55`,
    `PREFIX_AUTHOR_FLOOR 0.8` so changing a documented threshold breaks the gate.
- `scripts/check.sh` runs the suite after `npm test`
  (`python3 -m unittest discover -s scripts -t scripts -p 'test_*.py'`), using
  `$NOVELFORGE_AGET_PYTHON`, then the aget venv, then `python3`. CI installs
  `click`/`rich` and runs the same command, because `aget.py` is a CLI module.

### Defect found by the fixture

A French `Fourth wing` scored **0.90** for an English request: the language
weight (0.10) reduced an otherwise exact title but did not reject it, so the
first foreign copy could be downloaded when the English one was absent
(`choose_candidates` returns rows best-first). `candidate_score` now returns 0
for any row that declares a non-English language when no `--lang` was requested,
and for a declared mismatch when `--lang` was requested. An explicit
`--lang French` still selects the French edition.

### Acceptance criteria

- [x] A saved LibGen page plus expected metadata for positive cases and decoys.
      Two real pages are committed (`fourth-wing.html`, `dune.html`).
- [x] The selected row is asserted, and the decoy-only set is rejected without
      writing a file.
- [x] Title-only (Anna's) rows, fuzzy-title rejection and Ocean heading cleanup
      are covered.
- [x] Every NF-030 guard is pinned by a test, including `choose_candidates`
      threshold/order/limit behaviour and the documented threshold values.
- [x] Wired into `scripts/check.sh`, Python stdlib only, no network.
- [x] The suite exposed a real bug (foreign-language row at 0.90) and the engine
      was fixed rather than the assertion.

---

## NF-032 — Lawful public-domain book sources (Project Gutenberg, Standard Ebooks)

**Status:** `[x] DONE` (2026-11-16) — `gutenberg` and `standardebooks` are
explicit sources in `scripts/aget.py`, Books source cards and the Rust
allow-list; both deliver a verified EPUB end to end and are absent from
`auto`.  
**Priority:** P2  
**Area:** `scripts/aget.py`, `src-tauri/src/process.rs`, Books source cards,
Guardrails, `scripts/test_aget.py`, fixtures

### Why

NF-030 measures and documents the four free sources the user approved
(`libgen → welib → annas → ocean`). All four are shadow libraries, so the
licensing boundary in this document still has no lawful default path. The user
explicitly kept the current order and parked the distribution work (NF-024),
which is recorded as a product decision in NF-030. This item is the separate
follow-up: add sources that distribute the file lawfully.

### Product decision (2026-11-16)

The user accepted these sources **only as an explicit manual toggle**: their
catalogues are mostly public-domain and older works, not the current
fantasy/new-release market, so they must not dilute the automatic order. The
Books page therefore presents them as `Public domain · opt-in` cards, shows an
opt-in note when one is selected, and tells the user to search the catalogue
again after switching. `auto` still resolves to
`libgen → welib → annas → ocean` only.

### What was added (2026-11-16)

- `scripts/aget.py`:
  - `PUBLIC_DOMAIN_SOURCES = ("gutenberg", "standardebooks")` and
    `ALL_SOURCES`; `resolve_sources("auto")` and `resolve_sources("")` still
    return `SOURCES` unchanged, so the opt-in sources cannot leak into Auto.
  - `--source` accepts both new values; the branch chain dispatches to
    `gutenberg_download` / `standardebooks_download`.
  - Pure parsers `parse_gutenberg_search`, `parse_gutenberg_book`,
    `parse_standardebooks_search` so the saved fixtures test the real selectors
    without a network call.
  - `language_name()` maps `en`/`en-GB` and friends to English names;
    `is_english()` also accepts `English (US)`–style labels so a Gutenberg
    `en` page and a catalogue `English` request line up.
  - `payload_matches_format()` + `save_verified_payload()` refuse bytes that
    are not `PK\x03\x04` (EPUB) or `%PDF` before anything is written.
  - Gutenberg: HTML search on `gutenberg.org/ebooks/search/` (Gutendex was
    rejected as flaky — it timed out 3 of 5 probes), then the book page is read
    to confirm `dcterms:language` and to pick the `.epub3.images` link. A
    language mismatch, a low score or an HTML body fails closed.
  - Standard Ebooks: HTML search on `/ebooks?query=…`, then the book page to
    choose the plain EPUB. The plain `/downloads/<slug>.epub` URL answers with
    an XHTML interstitial that meta-refreshes to the same path with
    `?source=download`; `standardebooks_download_url` appends that parameter
    directly, prefers the plain EPUB over `.kepub.epub` and `_advanced.epub`,
    and falls back to a constructed `/downloads/<author>_<title>.epub` URL.
- `src-tauri/src/process.rs`: source allow-list extended to `gutenberg` and
  `standardebooks`, with the error text updated. New regression test
  `book_source_accepts_public_domain_sources` asserts both values produce
  `--source <name>`.
- `src/types.ts`: `BookSource` union extended with both ids.
- `src/pages/BooksPage.tsx`: two new source cards (`Public domain · opt-in`),
  `PUBLIC_DOMAIN_SOURCES`, an opt-in note rendered only while one is selected,
  and the header hint renamed to “Auto order: …”.
- `src/pages/GuardrailsPage.tsx`: new rule card “Public-domain sources are
  opt-in”, stating they never join Auto and only affect the next Fetch.
- `src/pages/BooksPage.test.tsx`: “uses an explicitly selected public-domain
  source” proves selecting Project Gutenberg surfaces the opt-in note and
  passes `bookSource: "gutenberg"` with the catalogue title/author.
- Fixtures + tests: real pages saved 2026-11-16 under
  `scripts/fixtures/gutenberg/` (`pride-search.html`, `pride-1342.html`,
  `les-miserables-135.html`) and `scripts/fixtures/standardebooks/`
  (`pride-search.html`, `pride-book.html`, `pride-download-interstitial.html`),
  each with a `README.md`. `scripts/test_aget.py` grew to **31 tests** with a
  `PublicDomainSources` class: auto-order exclusion, search parsing and
  selection, book-page language/URL extraction, language-name mapping,
  non-English rejection, the `?source=download` interstitial bypass, plain-EPUB
  preference, the constructed-URL fallback and HTML-payload rejection.

### Live evidence (2026-11-16)

| Probe | Command | Result |
| --- | --- | --- |
| Gutenberg, illustrated classic | `aget "Pride and Prejudice" --source gutenberg --budget 90` | 1.00 match, **24,864,480-byte** `Pride and Prejudice.epub` in 96.0 s |
| Gutenberg, app E2E | QA harness, *The Time Machine* / H. G. Wells | 4.5 s, 307,155-byte EPUB, `verified=true` |
| Standard Ebooks, app E2E | QA harness, *Pride and Prejudice* / Jane Austen | 4.6 s, 831,932-byte EPUB, `verified=true` |
| Standard Ebooks series layout | QA harness, *Barchester Towers*, Trollope, series *The Chronicles of Barsetshire* #2 | `…/Anthony Trollope/The Chronicles of Barsetshire/02 - Barchester Towers/02 - Barchester Towers.epub`, 912,931 bytes, verified |
| Gutenberg misses a modern title | `Dune` / Frank Herbert | best match 0.00, no file written, skipped in 1.8 s |
| Standard Ebooks misses a modern title | `Dune` / Frank Herbert | no result, no file written, skipped in 0.9 s |
| EPUB container audit | both downloaded EPUBs | `mimetype` is the first entry, stored, `application/epub+zip` |

The 24.8 MB Gutenberg file shows why the per-source budget stays soft on a long
transfer: the EPUB3 illustrated edition is mostly images and took ~96 s with
`--budget 90` because the transfer itself outlived the deadline check.

### Trade-off recorded

Gutenberg EPUB3 illustrated editions can be tens of megabytes; Standard Ebooks
files are ~0.8 MB and faster. Neither catalogue contains modern genre fiction,
which is exactly why the user asked for them to stay opt-in.

### Acceptance criteria

- [x] Both sources are selectable in Books and documented in Guardrails.
- [x] A public-domain title downloads end to end and shelves under the NF-029
      layout with a verified EPUB.
- [x] A missing/HTML response is rejected without writing a file.
- [x] The default order and NF-024 decision are unchanged unless the user says
      otherwise.

---

## NF-033 — NovelFire chapters 404 after an escaped mirror redirect

**Status:** `[x] DONE` (2026-11-16)  
**Priority:** P1  
**Area:** lncrawl-scraper redirect parsing, `scripts/patch_lncrawl_scraper.py`,
install gate

### Root cause (isolated, 2026-11-16)

The chapter list was never corrupted: every row in the app's sqlite held a
correct `https://novelfire.net/book/reverend-insanity/chapter-N` URL. The bad
URL was built at request time:

1. `novelfire.net` sometimes answers a chapter request with **302 →
   `https://readnovel.site/novel/reverend-insanity/chapter-7`**.
2. `readnovel.site` answers **200** with a 2,336-byte “Loading...” stub whose
   only payload is
   `window.location.href = "https:\/\/novelphoenix.com\/novel\/reverend-insanity\/chapter-7?noti=1"`.
3. lncrawl-scraper 1.7.0's `js_redirect` extracts the quoted string **without
   decoding `\/`**, and `urljoin()` resolves it against the readnovel page:

```text
https://readnovel.site/novel/reverend-insanity/\/\/novelphoenix.com\/novel\/reverend-insanity\/chapter-7?noti=1
```

4. That path is a permanent 404. Retrying re-follows the same redirect and gets
   the same 404, which is why 2/10 and 5/20 chapters vanished.

`https://novelphoenix.com/novel/reverend-insanity/chapter-7?noti=1` serves the
chapter (200, 86,213 bytes), so the mirror hop is valid once the destination is
decoded.

### Fix

`scripts/patch_lncrawl_scraper.py` patches the installed
`scraper/diagnosis.py` to return
`found.group(1).replace("\\/", "/")`, and the installer runs `--verify` against
the saved mirror page. The patcher is idempotent and refuses to guess if the
expected source line is missing.

The chapter-selection and offline-source-index changes now also ship as
repo-owned patches (`lncrawl-v4.14.0-chapter-selection.patch`,
`lncrawl-v4.14.0-offline-source-index.patch`), so a fresh checkout no longer
depends on working-tree edits.

### Evidence

| Run | Result |
| --- | --- |
| Isolation, ch 7–8, fresh data dir, no throttle | 2/2, 160 s |
| **After fix**, ch 7–8, `--rate-limit 2` | **2/2, 39 s**, no 404 |
| **After fix**, ch 1–20, `--rate-limit 2` | **20/20, 342 s** (same source, same throttle as the 15/20 failure); 299.9 KB EPUB, no `readnovel.site`/`novelphoenix.com` error |
| **After fix**, app QA, ch 7–8 | **2/2, 293 s**, complete 2,334-chapter index, `metadata.json` + `index.json` + `chapters.html` + `cover.jpg`, 74,870-byte EPUB `verified=true` |
| Patcher | 4 offline tests (`scripts/test_lncrawl_scraper_patch.py`), `--verify` passes |
| Startup | no GitHub source-index sync; `Sync online sources` no longer appears, removing ~90 s and the `Exhausted` loader traceback |

Original failing measurements are retained for comparison: 10 chapters 8/10
(7, 8), 20 chapters 15/20 (7, 8, 9, 14, 18).

### Log / failure message

The app used to report only `Process exited with code 1`, while the reason lived
in the streamed UI log. `process.rs` now keeps the last 24 stderr lines and
appends the headline failure (`Exhausted:` / `HTTPError` / `Traceback`, else the
newest error-like line) to the failure message. Example:
`Process exited with code 1 · Exhausted: no detection layer for … gave up after 5 attempts …`.
Two Rust tests cover the selection and the fallback.

### Follow-up

The app retry pass still re-fetches a chapter that genuinely 404s. That is now
an honest source failure rather than a URL bug; it is not part of NF-033.

---

## NF-034 — Licensed manga titles are external-only on MangaDex

**Status:** `[x] DONE` (2026-11-16)  
**Priority:** P2  
**Area:** `src-tauri/src/process.rs` job messaging, Guardrails, QA matrix

### Evidence (live probe, 2026-11-16)

Solo Leveling (`32d76d19-8a05-4db0-9fc2-e0b0648fe9d0`) reports
`availableTranslatedLanguages` including `en` and `lastChapter: 200`, but the
English aggregate now holds **16 chapters**. A single-chapter range probe
returns:

```text
[mangadex][error] Chapter 1 is not available on MangaDex and can instead be read
on the official publisher's website at https://br.comikey.com/read/solo-leveling-webtoon/…/episode-1/.
[mangadex][error] Chapter 1 is not available on MangaDex and can instead be read
on the official publisher's website at https://www.webnovel.com/comic/…
[mangadex][error] Chapter 1 is not available on MangaDex and can instead be read
on the official publisher's website at https://www.tappytoon.com/en/chapters/…
[mangadex][error] Chapter 1 is not available on MangaDex and can instead be read
on the official publisher's website at https://www.pocketcomics.com/comic/…
exit=4
```

Four language editions print one line each for the **same** chapter, and
gallery-dl still wrote two images before giving up: licensing is per edition, so
a title is rarely all-or-nothing. Previously the job only showed
`Process exited with code 4`, which reads as a crash.

The same title is complete on Weebcentral (201 chapters, measured at ~10 s and
~11 MB per chapter), so the app can still deliver it once the user pastes that
URL — the missing piece was telling them why MangaDex cannot.

### Fix

`ErrorTail` now parses the licensing line as it arrives and keeps a
`BTreeSet` of chapter numbers, the first publisher URL and the set of publisher
hosts. Counting happens on arrival, not in the 24-line ring buffer, so a
200-chapter series is summarised correctly (test:
`mangadex_licensing_counts_far_past_the_line_buffer`). One chapter across four
languages counts once.

The note replaces the exit-code message on failure, is logged to the dock, and is
kept on success so a partial CBZ is never presented as complete:

```text
MangaDex licensing: chapter 1 is external-only and must be read at
https://br.comikey.com/… NovelForge never scrapes the publisher page and keeps
any chapters that were available. Try another extractor (for example a
Weebcentral URL) if the series is available there.
```

Multiple publisher hosts are listed (up to three) instead of picking one
arbitrary URL. The official page is named, never requested.

### Verification

| Check | Result |
| --- | --- |
| `process::tests::mangadex_licensing_names_one_external_chapter` | one line → “chapter 7 is”, publisher URL preserved |
| `process::tests::mangadex_licensing_counts_far_past_the_line_buffer` | 40 lines → “40 chapters are”, ring buffer still 24 lines |
| `process::tests::unrelated_errors_do_not_claim_a_licensing_gap` | HTTP 503 keeps its own summary, no licensing claim |
| Live `gallery-dl --range 1` probe | `exit=4`, four external-only lines, two images written |
| Guardrails | “Manga availability is licensing, not failure” rule added to the Integrity policy |
| QA matrix | NF-034 row and the live recipe in §2 |

### Non-goals

- No automatic extraction from the publisher page; that is the licensed copy.
- Manga search stays direct-URL only, so the note names a URL pattern rather
  than resolving Weebcentral automatically (a follow-up if it earns its keep).
- No retry of the licensed chapters: retrying cannot make them available.

---

## NF-035 — A slow source can stall a crawl with no bound and no zero-output retry

**Status:** `[x] DONE` (2026-11-16)  
**Priority:** P2  
**Area:** `src-tauri/src/process.rs`, webnovel transport budget

### Evidence (2026-11-16)

- A `--rate-limit 2` run of NovelFire chapters 1–10 (fresh data dir) printed no
  progress for **700 s** and was killed by the harness. The scraper's request
  budget is `(15, 120)` seconds and lncrawl walks the 24 TOC pages serially, so
  consecutive read stalls can exceed twelve minutes before the first chapter.
- The app shows `idle` seconds but never fails a stalled crawl, so the job looks
  alive indefinitely.
- The crashed app job needed two timeouts to fail (GitHub source index ≈ 90 s,
  novel page ≈ 90 s) and exited with 0 files; nothing retried it. NF-033 removed
  the first timeout and the error-tail work now names the second, but a
  zero-output transient still requires a manual retry.
- 2026-11-16 15:28–15:33 UTC, NovelPhoenix: two app runs failed in **92 s** with
  `Exhausted: no detection layer … transport failure (Timeout)` after five
  15 s attempts (1/2/4/8 s backoff), exit 1, **zero output files**. The same page
  answered `curl` in 1.1 s and `lncrawl` directly succeeded six minutes later, so
  this is the zero-output transient the automatic retry is meant to absorb — it
  failed closed cleanly, but the user had to start it again.

### Scope

- Add a watchdog: if no output, no bytes and no phase change for a bounded
  interval, terminate the child with `Source stalled after …` and keep the work
  directory for inspection.
- Retry once, automatically, when the crawl exits non-zero with 0 artifacts and
  the user did not cancel; reuse the existing retry log/progress path.
- Never auto-retry validation failures (bad URL, missing title, incomplete
  index) — those must keep failing closed.

### Implementation (2026-11-16)

- **Heartbeat.** `JobProgress` already resets on every crawler log line, every
  byte of growth in the work tree, and every resume. That is the watchdog's
  clock, so "static UI" and "hung process" are the same condition.
- **Bounds.** `stall_limit_seconds()`: **900 s** while details/index work is in
  flight, **300 s** once chapters or images are flowing (`STALL_POLL_SECS = 15`).
  Paused time is subtracted, so resuming a job that sat paused for an hour does
  not trip the watchdog before the engine prints anything.
- **Action on trip.** `supervise()` kills the process group (SIGTERM, then
  SIGKILL), reaps it, drains both pumps and returns `Stalled { idle, limit }`.
  `run_job` fails the job with `Source stalled: no output, bytes or progress for
  … (limit …). Work directory kept.` and leaves every artifact in place.
- **One retry, by failure kind.** `CrawlSpawner` keeps program/prefix/args/cwd/
  work-dir so the engine can be launched again. `retry_action()`:
  - `Stalled` → **Resume**, regardless of partial output (`--all` → `--resume`);
  - non-zero exit, **zero artifacts**, transient stderr (`ErrorTail::
    is_transient()`) → **Restart** with identical args;
  - anything else (validation failure, cancellation, a second attempt, a
    non-webnovel engine) → no retry.
  `ErrorTail::is_transient()` requires a transport/rate-limit marker
  (`timeout`, `transport`, `connection`, `exhausted`, `429`, `503`, `1015`,
  `cloudflare`, `unreachable`, …) and lets explicit fatal markers
  (`unsupported url`, `invalid url`, `no crawler`, `not found`, `404`, `no
  chapters`) win. An empty tail fails closed.
- **State across attempts.** On retry the job record's pid is retargeted,
  progress is reset to `FetchingDetails`, `idle` is touched, and the error tail
  and artifact capture start clean. The poller and the live structurer keep
  running across attempts and stop before `finalize`.
- **UI.** A running row appends `· no data for mm:ss` once `idleSeconds >= 10`,
  so a motionless bar explains itself before the watchdog fires. The rail
  already showed `idle`.
- **Debug-only QA hook.** `NOVELFORGE_QA_STALL_SECS` shortens the bound and poll
  in debug builds so the fault-injection recipe below can prove the kill/resume
  path; release builds compile the hook out.

### Evidence (2026-11-16)

- Rust: `process::stall_tests` (9 tests): bound by phase; pause time excluded;
  a **real** `sleep 30` child is killed and reported `Stalled`; lone cover vs
  chapter/EPUB output; transient vs bad-URL tails; retry kind/attempt/engine
  gating; `--all` → `--resume` args. Full suite **172 lib tests + 2 offline
  integration tests passed**; frontend 29/29; Python 38/38.
- **Live good run** (app QA harness, real lncrawl, NovelPhoenix `1-2`):
  `Completed`, **2/2** chapters, 51,071-byte EPUB `verified=true`, crawl
  **44.6 s**, total **47.0 s**, no stall and no retry line.
- **Live fail-closed** (bad URL `https://example.com/not-a-novel.html`): one
  launch, failed in **2.5 s**, `retry —`, no automatic retry.
- **Live transient retry** (fault-injected engine: exit 1 + `Exhausted: …
  Timeout`, no artifacts): exactly **2 invocations**, one
  `retrying once from scratch (attempt 2/2)` line, final message
  `Process exited with code 1 · one automatic retry also failed · … Timeout`.
- **Live fatal control** (same stub, `No crawler found for the domain`): exactly
  **1 invocation**, `retry —`, message names the fatal reason.
- **Live stall + resume** (fault-injected engine writes one chapter body to the
  lncrawl store, then hangs; `NOVELFORGE_QA_STALL_SECS=3`): watchdog killed both
  attempts, logged `Stalled with partial output — resuming missing chapters
  (attempt 2/2)`, **2 invocations**, timing `crawl 5.4s · retry 5.4s`, message
  `Source stalled: … · one automatic retry also failed`, work dir kept.

### Follow-up

- NF-042: lncrawl prints fatal `No crawler found`/`Traceback` to **stdout** and
  exits 0, so the UI reports `Process finished without producing a usable output
  file` instead of the engine's reason; `ErrorTail` only reads stderr.

---

## NF-036 — NovelFire chapters cost three HTTP hops; the destination is one hop

**Status:** `[x] DONE` (2026-11-16)  
**Priority:** P1  
**Area:** `src-tauri/src/sources.rs` resolver preference

### Measurements (2026-11-16)

A NovelFire chapter is three requests in series:

```text
novelfire.net/book/<slug>/chapter-N     302 → readnovel.site       1.87 s
readnovel.site/novel/<slug>/chapter-N   200, 2,336 B JS stub        0.79 s
novelphoenix.com/novel/<slug>/chapter-N 200, real chapter           0.85 s
```

The scraper's pacer is start-to-start per origin and every hop is paced, so the
chain — not concurrency — sets the floor. Measured NovelFire run: **7.6 s/chapter**
(144.3 s for chapters 1–20, plus a 194 s TOC).

The destination serves the same chapters directly, in one request, on both
mirror hosts:

| Check | Result |
| --- | --- |
| `novelphoenix.net` TOC (`/reverend-insanity.html?page=1..59`) | **42.0 s**, 2,334/2,334 chapters, 59 pages, 0 retries, 0×429/503 |
| `novelphoenix.net` chapter | 200 direct, **0.89–1.38 s**, `#chapter-content`, 1,991–2,019 words |
| `novelphoenix.com` TOC (`?page=1..24`, 100 rows/page) | **43 s** at 1.2 s pacing, 2,334/2,334, 0 blocks |
| `novelphoenix.com` chapter | 0.34–0.80 s, `#content`, 1,978–2,736 words |
| Parity with the NovelFire copy of chapter 7 | **99.6 % token overlap**; only NovelFire's duplicated title header differs |
| Metadata / search on `.net` | `<h1>`, author present, `/search?keyword=` returns the title |
| Unpaced burst (24 pages, ~2.2 req/s) | Cloudflare **1015** after 13 requests; recovers at 1.2 s spacing. Scraper already maps 1015 → `Layer.BEHAVIOURAL` |

### Root cause: ranking, not a missing adapter

NovelPhoenix was already a verified source with a working patched adapter, and
the resolver already found it as an alternative. It lost the tie because
`rank_for_host()` used **registry position**, and NovelFire sits one row earlier
in the profile list. The resolver therefore downloaded from the three-hop host
while the one-hop host was listed one line below it in the alternatives.

An earlier probe of `novelphoenix.net` used the `.com` URL scheme
(`/novel/<slug>`), which 404s on `.net`; the real `.net` scheme is
`/<slug>.html`. That mistake is why this was briefly recorded as needing a new
adapter theme.

### Fix

`src-tauri/src/sources.rs` adds an explicit `PREFERENCE` order used only for
tie-breaking. `search_sources()` — the UI and search order — still returns
registry order. NovelPhoenix now outranks NovelFire; unknown hosts still rank
`u8::MAX`; unlisted known profiles keep registry order behind the listed ones.

### Verification

| Run | Result |
| --- | --- |
| `sources::tests::measured_cheaper_source_wins_the_tie_break` | NovelPhoenix ranks 0, NovelFire lower, every known host finite, unknown `u8::MAX` |
| `NOVELFORGE_QA_RESOLVE="Reverend Insanity"` | picks `novelphoenix.net`, confidence 1.00; NovelFire becomes an alternative |
| `NOVELFORGE_QA_INDEX_URL` (fresh) | 2,334 chapters, `complete=true`, **42.0 s**, 59 pages, 0 retries |
| lncrawl, 59-page TOC + chapters 1–20 | **20/20 in 56 s**, 281.6 KB EPUB (was 342 s on NovelFire) |
| App QA, title query, chapters 1–20 | **20/20, crawl 54.5 s, total 57.0 s**, 288,310-byte EPUB `verified=true`, cover + `metadata.json` + `index.json` + `chapters.html` |

Cold-start projection: `42 s index + ≈54 s crawl per 20 chapters` — the
per-chapter cost is now the pacer gap, not redirect latency. Extrapolating the
20-chapter run gives roughly 0.8 s/chapter; NF-037 is the full-length check.

### Non-goals

- No attempt to bypass the mirror chain's JS redirect. NF-033 already made it
  correct; this makes it unnecessary when a direct host exists.
- No change to the completeness gate. The full TOC is still required.
- No rate increase. The 1015 result shows the host notices bursts.

---

## NF-037 — Full-length single-hop run is projected, not measured

**Status:** `[x] DONE`  
**Priority:** P3  
**Area:** live verification only

NF-036 measured 20/20 chapters (56 s) and a full 2,334-chapter index (42 s), but
not a complete 2,334-chapter download. The remaining unknown is sustained load:
an unpaced 24-page burst earned Cloudflare **1015** after 13 requests, while the
paced index run at `--rate-limit 2` saw none.

Scope:

- Run one complete NovelPhoenix title at the shipped throttle and record total
  time, 429/503/1015 counts, and whether the pacer's behavioural backoff kept the
  run moving.
- Confirm the completeness gate still holds at 2,334/2,334 and the EPUB verifies.
- If 1015 appears mid-run, record the observed cadence and lower the default
  chapter rate for this host rather than adding retries.

Do not run this as a background experiment while other work needs the network;
it is expected to take ~30–60 minutes.

### Result (2026-11-16, *Reverend Insanity*, novelphoenix.net)

Two full runs were made. **Run B is the clean measurement**; run A shared the
machine with the NF-038 compilation/test work, so its rate is not representative.

| | Run A (13:11–14:27 UTC, killed) | Run B (14:35–14:54 UTC, fixed binary) |
| --- | --- | --- |
| Chapters | **2,334 / 2,334** | **2,334 / 2,334** |
| Crawl (child) | 2,326 s (~1.0 ch/s) | **1,113.8 s (~2.1 ch/s)** |
| TOC inside the crawl | 59 pages | 43 s, 24 volumes, 2,334 chapters |
| Index (app) | 42 s | 0.0 s (cached from run A) |
| Audit | 0.1 s | 10.1 s |
| Export | **never finished** (>37 min stalled) | **16.0 s** |
| Verify | — | 10.0 s |
| Total | killed after 50 min | **1,152.6 s (19m 12.6s)** |
| 429 / 503 / 1015 | 0 / 0 / 0 | 0 / 0 / 0 |
| Retries | 0 | 0 |
| Empty chapters | 0 | 0 |
| EPUB | 33,718,857 B, valid | 33,718,857 B, valid |

Run B transport line: `NovelPhoenix · 2 request(s) · 0 retries · 0×429 · 0×503 ·
0 blocked · avg 1635 ms · concurrency 1/3 · gap 700 ms · rate 2/s`. EPUB audit:
2,368 entries, `mimetype` first and exact, 2,362 XHTML, 2,334 chapter files, none
under 1 KB, `metadata.json` `complete: true`, `verified.ok: true`,
`chaptersExpected/chaptersFound 2334/2334`, `hasCover: true`.

Conclusions:

- The shipped `--rate-limit 2` sustains 2,334 chapters with **no Cloudflare
  challenge**, so the default is not lowered. The earlier 1015 came from an
  *unpaced* burst (24 pages, no gap), which is not what the app sends.
- Chapter crawl is ~96.6 % of wall clock; index, audit, export and verify
  together are ~38 s. The lever is hop count (NF-036), not more concurrency.
- The EPUB and completeness gate hold at full length.
- Run A's slower rate is attributed to concurrent builds/tests on the same
  machine, not to the source: run B hit the configured cap exactly.

---

## NF-038 — Full-length export recompiles a regex 5.4 M times and hangs the job

**Status:** `[x] DONE`  
**Priority:** P2  
**Area:** `src-tauri/src/export.rs` title matching, job phase reporting

### Evidence (NF-037 run A, 2026-11-16)

The 2,334-chapter download finished at 2,326 s with the completeness gate passed
(`Chapter audit · 2334/2334 chapters present and non-empty`) and all 2,334
chapter XHTML files plus the 33.7 MB EPUB on disk. The job then sat in
`Exporting 100 %` for **more than 37 minutes** with no further output: no
`metadata.json`, no `cover`, no `index.json`, no `chapters.html`. One tokio
worker had accumulated **>1,000 s of user CPU** while the process held ~50 % of a
core. A 20-chapter run never shows this because the cost is quadratic.

### Root cause

`normalize_title()` compiled a fresh `Regex` on every call:

```rust
let stripped = Regex::new(r"(?i)\bchapter\s*#?\s*\d+\s*[-:–—]?\s*")
    .expect("valid title prefix regex")
    .replace(&lower, "");
```

Two passes call it inside a nested scan:

- `export_epub` matches EPUB chapters to index entries by title to repair
  numbering gaps: `audit.chapters.len() × index.chapters.len()` calls =
  **5,446,356** regex compilations for this title.
- `merge_previous_export` repeats the same shape for every preserved chapter on
  a library update.

The chapters folder is written before that pass, so the job looks finished on
disk while the UI is stuck at 100 %.

### Fix

- `normalize_title` now compiles the pattern once in a `OnceLock`.
- `export_epub` normalises each index title and each audit title once, then
  compares the cached strings; the archive-path → file lookup is also a map
  instead of a linear scan.
- `merge_previous_export` precomputes normalised titles once per update and
  indexes them by position.

Behaviour is unchanged: same first-match semantics, same gap repair, same
preserved-chapter rules. Existing export/merge tests cover those paths.

### Verification

- `cargo test --lib export` (existing title and merge tests) passes.
- Full-length re-run (NF-037 run B, fixed binary): **export 16.0 s** for 2,334
  chapter files + 36 MB of split output, against a run that had been stuck at
  `Exporting 100 %` for **more than 37 minutes** before it was killed. A
  20-chapter smoke run exports in **0.2 s**.
- The job now reaches `Completed`: `metadata.json` (717,580 B), `index.json`
  (716,824 B), `chapters.html` (743,488 B) and `cover.jpg` (13,866 B) are all
  written, and the QA line reports `finished status=Completed`.
- The remaining 16 s is real I/O: reading a 33.7 MB EPUB, copying assets and
  writing 2,334 files. It no longer scales quadratically.

---

## NF-039 — Structure chapters while the crawl is still running

**Status:** `[x] DONE`  
**Priority:** P2  
**Area:** `src-tauri/src/structurer.rs`, `process::run_job`, `export::export_epub`

### Why

NF-038 removed the export stall, so the end phase is now seconds. That does not
fix the other half of a long run: until the crawl ends, the destination folder
contains nothing but the lncrawl artifact. A 2,334-chapter download is therefore
unreadable for ~39 minutes, and a crash/kill loses every structured chapter even
though lncrawl has already written all the bodies.

lncrawl stores each chapter body the moment it is extracted:

```text
<work>/novels/<id>/chapters/000123.zst     # zstd frame, serial = TOC position
```

Measured on the killed NF-037 run: `000001.zst` at 17:15:11, `002334.zst` at
17:41:58, i.e. the files arrive throughout the crawl (NF-037b showed 786 files
after ~7 minutes).

### Design

`structurer::spawn` starts a blocking poller for `Engine::Webnovel` jobs with a
private work dir. Every 500 ms it lists `work_dir/novels/*/chapters`, and for
each new `<serial>.zst`:

1. decode the frame with `ruzstd` (`StreamingDecoder`, capped at 8 MB),
2. clean it with the same `ChapterCleaner::for_source` the export uses,
3. write `chapters/<serial> - <title>.xhtml` through a dot-prefixed temp file
   and `rename` (atomic, so a reader never sees half a chapter).

Titles come from the body (`h1`–`h3`, else a leading bold `Chapter N – …`
paragraph, else `Chapter <serial>`); the serial is what the final export
matches on, never the provisional title.

Chapters observed mid-write fail frame validation and are retried when the file
changes (size + mtime), so no truncated chapter is ever written.

### Interaction with the final export

- The poller stops and is joined immediately before `finalize`, so it cannot
  write after the export sweep.
- `export_epub` writes the authoritative `<serial> - <EPUB title>.xhtml` files,
  then `structurer::sweep` removes provisional names whose serial now has a
  different export file.
- A chapter the export could not produce (`file: None`) keeps its provisional
  file, so a partial update can never delete chapters recovered from a crashed
  crawl.

### Verification

- Offline: 5 tests — 4 in `structurer::tests` using a real saved NovelPhoenix
  chapter (truncated excerpt, compressed with lncrawl's `zstd.compress(_, 10)`):
  live structuring, mid-write retry, translation/`.zst` filtering, and sweep
  safety; plus `chapter_clean::tests::void_elements_are_self_closing_so_the_xhtml_parses`.
- Live (2026-11-16 15:39–15:40 UTC, 40 chapters, NovelPhoenix): provisional files
  appeared **while the job was still `DownloadingChapters`** and grew in step with
  the store — `files=5 zst=7` at 30 %, then 14, 22, 31, 40. Log:
  `Live structuring · 1 chapter file(s) ready while the crawl continues · latest
  0031 - Chapter 31- Fang Yuan You're in huge trouble.xhtml`, then
  `Live structuring · 40 chapter file(s) ready before the final export`.
  Final state: `Timing · crawl 1m 04.3s · metadata 1.3s · index 0.0s · audit 0.2s
  · retry — · export 0.4s · verify 0.2s · total 1m 06.7s`, `Completed`, 40/40.
- After export: exactly **40** chapter files + `style.css`, **0** provisional
  leftovers, no duplicate serials, no `.part` files, and `xmllint --noout` accepts
  every final chapter.

### Known limits

- The library entry is written with the export, so a crash leaves readable
  chapter files but no `metadata.json`; a re-run re-indexes them.
- Images referenced by a provisional chapter are copied by the export, so they
  may 404 until the export runs.

---

## NF-040 — A throttled origin never recovers its pacing interval

**Status:** `[x] DONE`  
**Priority:** P1  
**Area:** installed `scraper/pacing.py` (lncrawl-scraper 1.7.0), `scripts/patch_lncrawl_scraper.py`, `scripts/install-tools.sh`

### Why

A live NovelFire run (2026-11-16 20:13 local, `--all --rate-limit 2`,
2,334-chapter index, started from the novelfire.net URL and continuing a
3-chapter folder) was adding a chapter every **10–25 s**. The app looked
hung: 32 chapters in 13 minutes, then 7 chapters in 75 s (5.6 ch/min). The
same title on NovelPhoenix finishes ~3 chapters/s.

The cause was not the network or the machine. lncrawl-scraper keeps a
per-origin pacing profile in `~/.lncrawl/scraper/origins.json`:

```text
novelfire.net     interval 9.801  successes 401  failures 6   first 11-10 17:57
novelphoenix.net  interval 0.333  successes 218  failures 0
freewebnovel.com  interval 1.000  successes  75  failures 3
```

NovelForge asks for 2 requests/s (`1/rate_limit` = 0.5 s), but `Session` calls
`Pacer.learn(key, profile.interval)` **before every request**, and `learn()`
assigns unconditionally:

```python
self._interval[origin] = min(interval, self.policy.max_interval)
```

`eased()` narrows by 10 % only after ten consecutive successes, so the next
request put the widened profile value back. The profile's own decay is the
0.9/0.1 EMA in `record_success`, i.e. ~1 % per ten successes. A single throttle
on 2026-11-10 therefore still cost 9.8 s per request six days and 401 successes
later.

Simulating `Session`'s exact loop against the installed `Pacer` (starting at the
recorded 9.801 s, target 0.5 s) showed the ratchet barely moves:

| requests | stock profile |
|---:|---:|
| 10 | 9.703 s |
| 100 | 8.864 s |
| 500 | 5.930 s |

### Change

`scripts/patch_lncrawl_scraper.py` now also patches `Pacer.learn()`: an origin
already tracked in `_interval` keeps this run's fresher measurement; the
persisted profile is adopted only once, for an origin the pacer has not seen.
No rate is raised — `eased()` still requires ten consecutive successes per 10 %
step and never goes below the caller's `--rate-limit`. The patch is idempotent,
fails loudly if the expected body is missing, and `--verify` simulates the
ratchet on the installed package.

Same simulation after the patch:

| requests | patched profile |
|---:|---:|
| 10 | 9.703 s |
| 100 | 3.975 s |
| 200 | 1.386 s |
| 300 | 0.514 s |
| 400 | 0.500 s |

### Verification

- `scripts/test_lncrawl_scraper_patch.py`: 3 new tests (idempotent rewrite,
  missing-body rejection, exactly one store assignment behind the guard); 7/7
  pass.
- Live package: patch applied to the running venv, `--verify` reports
  `Pacer.learn keeps the narrowed interval (8.820s) and still adopts unseen
  origins`, and a second run is a no-op.
- The job running at the time keeps its already-imported module, so the fix
  applies from the next crawl; the recorded 9.8 s profile is still adopted at
  start and then eases down instead of sticking.
- Distribution: the patch lives in the tools installer
  (`scripts/install-tools.sh` → `patch_lncrawl_scraper.py`), the same path as
  NF-033. An upgrade that reuses an existing venv must re-run that installer;
  the app does not patch its tools at startup.

---

## NF-041 — An update reuses a slow host with no pace signal or faster option

**Status:** `[x] DONE (2026-11-16)`  
**Priority:** P1  
**Area:** `src-tauri/src/model.rs`, `src-tauri/src/export.rs`,
`src-tauri/src/process.rs`, `src-tauri/src/library_actions.rs`,
`src-tauri/src/qa.rs`, `src/pages/LibraryPage.tsx`, `src/pages/DownloadsPage.tsx`

### Why

An update reuses the library entry's stored `sourceUrl` (correct: never
silently change what the user downloaded). But an entry created before NF-036
still points at NovelFire, whose chapters cost a 302 + mirror stub + mirror page
and — until NF-040 — a persisted 9.8 s gap. Starting a new download from an
explicit NovelFire URL (a result row or a pasted link) takes the same path: it
continues the existing partial folder instead of resolving the title, so NF-036's
NovelPhoenix preference only applies to a *title* search. The Library shows the
host, yet nothing says that host is expected to run at ~6 chapters/min, and the
job card shows neither rate nor ETA. The result is indistinguishable from a hang.

### What shipped

- `CrawlPace { chapters, seconds, secondsPerChapter, chaptersPerMinute,
  measuredAt }` is stored as `metadata.json → crawl` by
  `export::record_crawl_pace()` after every webnovel export that actually wrote
  chapters. It is **measured**, never estimated: fewer than one chapter or under
  one second records nothing. An update records only the chapters its selection
  crawled (`crawled_chapter_count`), not the merged book total — the first live
  update exposed 53 merged chapters in 5.7 s and claimed 556 ch/min before this
  guard.
- `LibraryNovel` now carries `sourceHost`, `sourceRank` (registry preference,
  `u8::MAX` unknown) and `crawl`. The card shows
  `Measured 94.4 ch/min · 0.64 s/chapter` with the measurement date in its
  tooltip; folders without a measurement show nothing.
- An update/retry of a folder with a measurement sets
  `DownloadTask.expectedChaptersPerMinute` / `expectedSeconds`
  (`ceil(chapters / rate × 60)`), returned from `update_download` and emitted on
  the job row, plus the dock line
  `Measured pace · … chapters/min on <host> · N chapter(s) selected · expected crawl …`.
  The Downloads page renders `Measured … on this source · expected crawl …`.
- `suggest_faster_source(folder)` resolves the stored title and offers only a
  download-ready profile ranked **lower** than the stored host with
  `resolver::relevance >= 0.85`. The Library's `Faster source` button opens a
  comparison dialog; confirming calls the normal download with
  `DuplicateMode::Copy`, creating a new folder and **never** changing the stored
  entry. Rejecting keeps the current source. No automatic switch exists.
- Debug harness: `NOVELFORGE_QA_ACTION=faster` prints the report; the update
  action prints the annotated expected fields.

### Live evidence (2026-11-16)

- **Pace persistence:** NovelPing *Prison Of Eternal Sky* 53/53 in 33.7 s →
  `metadata.json crawl = { chapters: 53, secondsPerChapter: 0.64,
  chaptersPerMinute: 94.4 }`; `list_library` exposed `novelping.com`, rank `4`
  and the same crawl object.
- **Update inherits the pace:** after deleting two real chapter files, the
  update selected exactly chapters 20–21. It logged
  `expected pace=Some(94.38) ch/min expected_seconds=Some(2)`, completed in 8 s,
  restored 53/53 with no missing file, and rewrote `crawl` as
  `{ chapters: 2, secondsPerChapter: 3.26, chaptersPerMinute: 18.41 }` — the two
  chapters actually crawled.
- **Faster host offered:** the NovelFire *Reverend Insanity* folder (rank 5)
  resolved to `novelphoenix.net` rank 0, confidence `1.00`,
  note “Verified … measured cheaper than novelfire.net. Starting it creates a new
  library folder; the existing entry at novelfire.net is not changed.”
- **Already fastest:** the NovelPhoenix folder (rank 0) returned no alternative
  and “novelphoenix.net is already the fastest measured host. Its chapters are
  one request each; no measured-cheaper source exists.”
- Tests: `export::tests::measured_crawl_pace_is_persisted_and_exposed_to_the_library`,
  `process::tests::expected_pace_rounds_up_and_needs_a_measurement`,
  `process::tests::crawled_chapter_count_uses_the_run_selection_not_the_merged_export`.

---

## NF-042 — A fatal crawler error on stdout is reported as "no usable output"

**Status:** `[x] DONE (2026-11-16)`  
**Priority:** P3  
**Area:** `src-tauri/src/process.rs` (`pump_lines`, `ErrorTail`, `run_job`)

### Evidence (2026-11-16)

Live QA with `https://example.com/not-a-novel.html` (a direct source link that
passes validation): lncrawl printed
`lncrawl._server_errors.ServerError: Error(501): No crawler found for the domain`
and a traceback to **stdout**, then exited **0**. `pump_lines` sent stdout only
to artifact capture (not `ErrorTail`), and a zero exit goes to verification,
where it failed with
`Process finished without producing a usable output file.` The user never saw
the real reason, even though it was in the log dock.

### What shipped

- The stdout pump now feeds `ErrorTail::push_stdout()` too. `stdout_carries_reason()`
  keeps only `Exhausted:`/`Error:`/`HTTPError`/`Unsupported URL`/`No crawler`/
  `ServerError`/connection/SSL/timeout lines and drops `Traceback (most recent
  call last)`, `File "…"` frames, `During handling…`, and `[time] INFO/WARNING`
  progress rows.
- `summary()` no longer selects `Traceback (most recent call last)` as the
  headline; the final exception line is the reason. `is_transient()` still lets
  `no crawler`/`unsupported url`/`404` win, so a stdout fatal never retries.
- When lncrawl exits 0 with **no artifact**, `run_job` prefixes the failure with
  `Source error · <summary>`. A precise `Source error` message is no longer
  overwritten during verification or suffixed with the generic no-output
  sentence (`zero_output_message`).
- Tests: `stdout_traceback_names_the_actionable_reason`,
  `stdout_progress_and_info_lines_are_not_errors`,
  `stdout_exhausted_line_is_still_a_transient_failure`,
  `zero_output_message_keeps_a_precise_stdout_reason`.

### Live evidence (2026-11-16)

- Real `https://example.com/not-a-novel.html`: **1** lncrawl launch, crawl
  2.4 s, message
  `Source error · lncrawl._server_errors.ServerError: Error(501): No crawler found for the domain`.
- Fault-injection stub that prints a traceback + `ServerError` to stdout and
  exits **0**: **1** launch, no retry, same reason surfaced.
- Transient stderr stub still retried exactly **2** times and ended with
  `one automatic retry also failed · Exhausted: … transport failure (Timeout)`.
- Fatal stderr stub still stopped at **1** launch.

---

## NF-043 — A TOC that numbers early chapters leaves real names off the shelf, the index and the EPUB

**Status:** `[x] DONE (2026-11-17)`  
**Priority:** P1  
**Area:** `src-tauri/src/metadata.rs` (`fetch_chapter_index_for`,
`enrich_index_titles`, `enrich_chapter_titles`, `recover_chapter_titles`),
`src-tauri/src/export.rs` (`export_epub`, `retitle_epub`),
`src-tauri/src/sources.rs` (`SupportLevel::is_index_proven`),
`src/components/novels/NovelSearchPanel.tsx`

### Evidence (2026-11-17)

NovelPhoenix's table of contents for *Reverend Insanity* publishes a real name
from chapter 348 onward but leaves **379 of 2,334** rows as `Chapter 12`
(chapters 1–347 plus scattered rows). *Omniscient Reader's Viewpoint* leaves
**65 of 551** rows numbered (0–61 and 276–278). *Lord of the Mysteries* leaves
only one row numbered, which is why it always looked correct.

A fresh index run reproduced it exactly:

```
[qa] index ok total=2334 complete=true source=novelphoenix.net
[qa] range 1..2334 first="Chapter 1" last="Chapter 2334 (END) - …"
```

The name is not on the host: the TOC link title, the chapter `<h1>` and the
JSON-LD all say `Chapter 1`. It must come from somewhere else. Novelfire mirrors
both books and publishes the full list, but its ORV TOC has the **same** 65
gaps, so the early ORV names exist only in the chapter page's own text
(`Episode 1 – Starting the Paid Service (1)`, `Prologue – …`).

### What shipped

- `fetch_chapter_index_for` runs a title pass after a complete index is built
  and also repairs an existing cache entry in place (no TOC re-walk).
- **Donor pass** (`enrich_chapter_titles`): the target's own title/author is
  read from its detail page, an exact-normalised-title search runs, and a
  candidate is accepted only when the host's profile has a proven index
  (`Verified` or `IndexVerified`), the normalised title is an exact match and
  both sides' authors agree when both are known. One hop only
  (`allow_enrichment=false`) so two numbering-only sources cannot bounce names.
- `adopt_chapter_titles` fills **only bare rows**, matched by the visible
  chapter number (`display_chapter_number`), never by URL order. Source-published
  names and every download URL/number stay untouched. It requires ≥95 % coverage
  of the bare rows and ≥2 adopted names, and `names_align_with_donor` rejects a
  shifted donor when ≥5 rows are named on both sides and <80 % normalised names
  agree.
- **Recovery pass** (`recover_chapter_titles`): for the rows no donor can name,
  read the chapter page itself and take the heading printed at the top of the
  content (`Episode N – …`, `Prologue – …`, or a short unlabelled name line).
  `looks_like_chapter_name` rejects prose (punctuation, leading quotes/brackets,
  word and length caps). Capped at 80 pages and 120 s, applied incrementally.
- Per-index flags `title_lookup_attempted` / `title_recovery_attempted` keep a
  failed lookup or a timed-out recovery from re-running on every cache hit; a
  timed-out recovery resumes with only the rows that remain numbered.
- **EPUB retitle** (`retitle_epub`): after the structured export, the bundled
  EPUB's `<title>`, `<h1>`, `nav.xhtml` links and `toc.ncx` labels are rewritten
  for every row whose index name differs from lncrawl's heading. The heading
  keeps the number (`Chapter 1 - The heart of a demon never has regret even in
  death`) so a later audit can still recover the serial. The archive is rebuilt
  into a temporary file, re-audited, and only then swapped in; a rewrite that
  changes nothing leaves the original bytes alone. Fully named books never enter
  this path.
- The search panel shows *“Chapter names borrowed from novelfire.net · this
  source leaves some chapters unnamed; names were matched by chapter number”*.

### Live evidence (2026-11-17)

- RI fresh index: `Filled 379 numbered chapter name(s) from novelfire.net`;
  chapter 1 → `The heart of a demon never has regret even in death`,
  chapter 2 → `Going back in time with 500 years of knowledge`, chapter 1168
  keeps its source name.
- ORV fresh index: 65 recovered from NovelPhoenix chapter pages — chapter 0 →
  `There are three ways to survive in a ruined world`, chapter 1 →
  `Starting the Paid Service (1)`, chapter 276 → `■■ (1)`; **0 bare rows**.
- LOTM: unchanged titles; its single bare row recovered; no donor caption.
- Live RI download (`chapters 1-2`, copy mode) → `metadata.json`, `index.json`,
  `chapters.html`, the XHTML filenames and the **EPUB internals** all carry the
  borrowed names; `complete=true`, `verified.ok=true`:

```
EPUB/chapter_00001.xhtml  <title>Chapter 1 - The heart of a demon never has
                                 regret even in death</title>
                          <h1>Chapter 1 - The heart of a demon …</h1>
EPUB/nav.xhtml            <a …>Chapter 1 - The heart of a demon …</a>
EPUB/toc.ncx              <text>Chapter 1 - The heart of a demon …</text>
```

### Tests

`bare_chapter_title_matches_numbering_only_titles`,
`donor_chapter_name_strips_repeated_numbering`,
`display_chapter_number_prefers_the_visible_number`,
`adopt_chapter_titles_matches_by_visible_number_and_keeps_existing_names`,
`adopt_chapter_titles_rejects_low_coverage_and_changes_nothing`,
`adopt_chapter_titles_rejects_a_shifted_donor`,
`title_lookup_runs_only_when_the_source_leaves_rows_unnamed`,
`chapter_header_names_are_recognised_from_content`,
`chapter_name_is_read_from_the_content_container`,
`borrowed_chapter_names_are_written_into_the_bundled_epub`,
`bundled_epub_is_untouched_when_every_row_was_named`.
`cargo test --lib` → **190 passed**.

---

## NF-044 — A visible chapter number is not lncrawl's serial on sources that start at chapter 0

**Status:** `[x] DONE (2026-11-17)`  
**Priority:** P1  
**Area:** `src-tauri/src/process.rs` (`engine_chapter_spec`, `build_args`,
`start_download` preflight, in-job retry), `src-tauri/src/export.rs`
(`plan_audit`, `export_epub`), `src-tauri/src/library_actions.rs`
(`full_chapter_spec`, `retry_chapter_spec`), `src-tauri/src/structurer.rs`
(`sweep`), `src/lib/chapters.ts`

### Evidence (2026-11-17, found during NF-043 live QA)

NovelPhoenix's ORV catalogue starts at `chapter-0.html` (visible *Chapter 0*,
the prologue). lncrawl numbers its TOC rows from 1, so visible `0` is serial
`1`. Requesting `--chapters 0-1` fetched **one** chapter and the run failed
closed:

```
$ lncrawl … --chapters 0-1 …
│ 6 volumes, 551 chapters
All selected chapters are already downloaded
[qa] Failure — Incomplete output — 1 of 2 chapters found
```

The app currently passes the visible numbers from the picker straight to
`--chapters`, so a partial ORV download is one chapter off (or fails the audit).
Full-book downloads are unaffected because no spec is passed.

### What shipped

- `metadata::fetch_chapter_numbers_for` fetches the catalogue **without** the
  title passes (`allow_enrichment=false`). A partial run only needs the row
  order, and the export's own index fetch afterwards reuses the same cache entry
  and enriches it, so total network work is unchanged.
- `process::start_download` preflights that index **before** creating the
  destination when a webnovel download carries a partial spec. A failed or
  timed-out index aborts before any folder or chapter exists (a reserved
  "Keep both" folder is removed again).
- `process::engine_chapter_spec` maps each displayed number to the 1-based
  catalogue serial by **row position**, then re-compresses the serials
  (`2-3` → `3-4` on ORV). It refuses an incomplete or empty index, a number that
  appears twice, and a number the list does not contain — it never falls back to
  the neighbour. `build_args` requires the index for a partial run and has no
  path that passes an unproven number to `--chapters`.
- In-job retries build their spec from the index's visible numbers and map it
  through the same function. If the index became unavailable, the retry is
  skipped and the partial output is kept instead of retrying with a wrong spec.
- `export::plan_audit` gained `misnumbered_positions`: when every fetched
  heading is numbered and strictly increasing, each heading must equal the
  selected visible number. This catches a count-only success where the spec was
  one row off. Unnamed headings and volume-local restarts keep the old
  count-only behaviour.
- `library_actions::full_chapter_spec(index)` now compresses the index's real
  numbers (`0-110,112-551` for ORV) instead of `1..=total`;
  `retry_chapter_spec` prefers the manifest's own numbers (so 0-based books
  retry the right rows) and only falls back to `1..=total` for a short manifest.
- `structurer::sweep` takes the serial → visible map, so a provisional live file
  named by lncrawl serial is compared with the export file for the **visible**
  chapter. Files the export wrote are always kept, and a chapter the export
  could not produce keeps its streamed copy. Without this, the ORV `2-3` run
  left a stray `0004 - Chapter 4.xhtml` beside the real `0003` file.
- The frontend `parseChapterSpec` accepts chapter `0` so a 0-based prologue can
  be selected; 1-based sources still reject it during the index check.

### Live evidence (2026-11-17)

ORV `2-3` (`/tmp/opencode/nf044-orv-2-3c.log`, copy mode):

```
[qa] job … resolved title="Omniscient Readers Viewpoint" destination=…/omniscient-readers-viewpoint-copy-3
[qa:sys] Chapter audit · 2/2 chapters present and non-empty
[qa:sys] Structured export · metadata.json + index.json + chapters.html · 2 chapter file(s) · cover.jpg
[qa:sys] Success — 1 verified output file(s).
chapters/0002 - Starting the Paid Service (2).xhtml   <h1>Chapter 2</h1>
chapters/0003 - Starting the Paid Service (3).xhtml   <h1>Chapter 3</h1>
EPUB/toc.ncx  <text>Chapter 2 - Starting the Paid Service (2)</text>
              <text>Chapter 3 - Starting the Paid Service (3)</text>
```

No provisional `0004` left behind, `metadata.json` names rows 2 and 3,
`verified.ok=true`, `complete=true`.

ORV `275-277` (`/tmp/opencode/nf044-orv-275.log`) — a range that spans named and
unnamed rows: 3/3, files
`0275 - Chapter 275 - Giant Story (4).xhtml`, `0276 - (1).xhtml`,
`0277 - (2).xhtml`; EPUB nav labels are `Chapter 275 - Giant Story (4)`,
`Chapter 276`, `Chapter 277`.

ORV `111` (`/tmp/opencode/nf044-orv-111.log`) — the TOC skips this visible
number. The run fails **before** any folder or crawl and the ORV folder count is
unchanged:

```
[qa] start failed: Chapter 111 is not in the source's chapter list, so the
     download was not started. The list shows 551 chapter(s).
folders before=4 after=4
```

RI `1-2` (`/tmp/opencode/nf044-ri-1-2.log`) — a 1-based source, so the mapping is
the identity and nothing regressed: 2/2,
`0001 - The heart of a demon never has regret even in death.xhtml`,
`0002 - Going back in time with 500 years of knowledge.xhtml`.

### Tests

`webnovel_selection_is_translated_from_visible_numbers_to_serials`,
`webnovel_selection_fails_closed_without_a_matching_index_row`,
`webnovel_selection_refuses_an_incomplete_or_ambiguous_index`,
`audit_rejects_a_selection_that_fetched_the_neighbouring_chapter`,
`sweep_maps_a_serial_to_the_visible_chapter_the_export_wrote`,
`full_spec_covers_every_listed_chapter`, plus three `parseChapterSpec` frontend
tests (chapter 0, negatives, mixed compression).
`cargo test --lib` → **195 passed**; `scripts/check.sh` green (195 lib + 2
integration + 35 frontend + 38 Python + production build).

---

## 5. Download speed: full explanation


### 5.1 The dominant factors

For text novels, the approximate time is:

```text
total_time ≈ index_time
           + (chapters / effective_concurrency) × (latency + source_delay + parse_time)
           + retry_time
           + export_time
```

Where:

- **latency** = network RTT + TLS + server processing,
- **source_delay** = intentional throttle/sleep/rate limit,
- **parse_time** = HTML parsing and chapter extraction,
- **effective_concurrency** = actual parallel requests the source tolerates,
- **retry_time** = 429/5xx/backoff time,
- **export_time** = XHTML generation, ZIP/EPUB compression, file writes.

### 5.2 Bandwidth is rarely the bottleneck

A chapter is small HTML. Even 2,334 chapters may be only tens of megabytes. At typical broadband speeds that is seconds. The time is spent waiting for request responses, not transferring bytes.

### 5.3 Hardware matters, but secondarily

Hardware matters for:

- HTML parsing and text extraction (CPU),
- XHTML generation,
- EPUB/ZIP compression (CPU, disk),
- writing thousands of files (disk IOPS),
- WebView rendering (GPU/CPU),
- reading large `metadata.json` files when scanning the library.

A slow HDD or low-end CPU can add noticeable time to a full-book export, but it will not make a throttled source faster. RAM is generally not a constraint for text novels.

### 5.4 Current app-specific facts

- Webnovel throttle is now wired to `--rate-limit 2` (NF-001 DONE).
- `runner_concurrency` (default 5) is the scheduler's **job** count, not per-chapter
  requests: it starts that many `JobRunner` threads, and each runs one job. It has no
  effect on the CLI path NovelForge uses.
- The CLI path already parallelises chapters: `checkpoint` submits every selected chapter
  to `Crawler.taskman` (`ThreadPoolExecutor`, `max_sessions_per_exit + 1` = **3** workers
  by default). Chapter fetches are not serial.
- Extra threads do not raise request throughput, because the scraper's pacer is
  **start-to-start per origin** (`Pacer.next_delay` subtracts time since the previous
  request *started*). Threads overlap parsing and waiting; they cannot beat one request
  per mean gap.
- The app’s own job queue is faster only because it is **lazy**: reading a chapter fetches
  one chapter, and the library keeps fetched bodies on disk. A reader who opens 3 chapters
  never pays for 2,334.
- Chapter fetching therefore uses lncrawl's source pacing, capped at 2 req/s when throttle
  is on and the source default (currently 3 req/s) when it is off.
- A *learned* per-origin interval can override that target. lncrawl keeps one in
  `~/.lncrawl/scraper/origins.json` and, before NF-040, re-applied it on every request,
  so `novelfire.net` was still at **9.801 s/request** on 2026-11-16 after 401 successes
  and 6 failures since 2026-11-10. One chapter is one request there, so a full update ran
  at ~6 chapters/min. `novelphoenix.net` had no failures and 0.333 s/request.
- NovelFire TOC is intentionally serialized: 24 pages, measured at **194 s** on
  2026-11-16 (server latency dominates; a fast run can finish in ~35 s).
- The 3-chapter live test took ~41 s, almost all of it index and import, not bandwidth.
- “Speed” in the UI now measures the isolated lncrawl work directory during crawl and the
  destination only during import/export/verify (NF-005 DONE).

### 5.5 Realistic expectations

Measured on 2026-11-16 (NovelFire, Reverend Insanity, shipped `--rate-limit 2`,
chapters 1–20): index **194 s**, then **144.3 s for 20 chapters = 7.6 s/chapter**,
total 342 s.

```text
full run ≈ 194 s + 2,334 × 7.6 s ≈ 17,930 s ≈ 5.0 hours
```

The theoretical 2 req/s lower bound (19.5 minutes) assumes a 500 ms request with
no retries and no redirects. Real chapter fetches cost 2–4 s of server latency,
NovelFire adds a 302 → mirror hop, and lncrawl paces requests start-to-start per
origin. The ceiling is never reached; expected range is **4.5–6.5 hours**
throttled. With throttle off, FreeWebNovel measured 4.0 s/chapter, about **2.6
hours** for this title.

### 5.6 Measured: where the 7.6 s/chapter actually goes (2026-11-16)

A NovelFire chapter is **three HTTP requests**, not one:

```text
novelfire.net/chapter-N        302 → readnovel.site  (~1.5–2.0 s)
readnovel.site/.../chapter-N   200, 2,336-byte JS stub (~0.8 s)
novelphoenix.com/.../chapter-N 200, the actual chapter (~0.8 s)
```

Each hop is paced independently, so the chain costs roughly `3 × (gap + latency)`
and no amount of worker threads can collapse it. Measured raw latencies for the
three hops: 1.87 s / 0.79 s / 0.85 s.

The destination serves the same chapters **directly** — one request, no stub:

| Measurement (novelphoenix.com, 1.2 s pacing) | Result |
| --- | --- |
| Full TOC, 24 pages × 100 chapters | **43 s, 2,334/2,334 chapters, 0 blocks** |
| 10 chapters (100–109) incl. 1.2 s sleeps | **17 s → 1.7 s/chapter** |
| Per-chapter latency | 0.34–0.80 s, 1,978–2,736 words |
| Content parity vs the NovelFire copy of chapter 7 | **99.6 % token overlap** (only NovelFire’s duplicated title header differs) |
| Burst test, 24 pages at ~2.2 req/s | Cloudflare **error 1015** after 13 requests; fine again at 1.2 s spacing |

The scraper already maps `1015` to `Layer.BEHAVIOURAL`, so it backs off instead of
retrying blindly.

```text
single-hop full run ≈ 43 s + 2,334 × 1.7 s ≈ 66 min
current 3-hop run   ≈ 194 s + 2,334 × 7.6 s ≈ 5.0 h
```

So the leverage is **hop count, not concurrency** — about 4.5× on this title.
Shipped as NF-036: the resolver now prefers NovelPhoenix over NovelFire when both
verify the same title (`sources::PREFERENCE`). Measured after the change: 20/20
chapters in **56 s** end to end (was 342 s).

Throttle-off and multiple parallel crawls do not remove the source's own
pacing, and hammering a host is what produces 429s and mirror redirects. Always
measure and watch 429/5xx rates rather than assuming a concurrency number.

### 5.7 Proxies: what the old scripts actually measured (2026-11-16)

Reviewed the September scripts in `~/Documents/Scraper/` and their
surviving output in `~/downloaded_novels/`. They are straightforward
thread pools, not a pacing model:

- `fast_scrape.py` — 8 threads, free HTTP proxies scraped from public lists
  (`get_proxies.py`), one proxy per request via `itertools.cycle`, **no delay**.
  A 429/403 permanently removes the exit; when the list empties it continues
  **direct with 8 workers and no pacing**.
- `tor_scraper.py` — 3 threads through one `socks5h://127.0.0.1:9050`, with a
  global start-to-start lock of 2 s, so the whole process can start at most
  **0.5 request/s**. There is no `NEWNYM` and no `IsolateSOCKSAuth`, so it is one
  exit identity, not “several sides”.

Both open a new `requests.Session()` per chapter, so every fetch pays a fresh
TCP + TLS handshake, and on NovelFire a third hop through the mirror stub.

What the artifacts show (NovelFire, 2026-09-07):

| Check | Result |
| --- | --- |
| `working_proxies.json` discovery (02:56) | **4** exits survived testing → 4 concurrent-friendly IPs for 8 workers |
| *Lord of the Mysteries* output (04:42) | 1,198 chapters written, range 17–1432, **234 gaps**, first 16 chapters absent |
| `progress.json` for that slug | **1,430** chapters marked completed |
| Silent loss | **232 chapters were marked done but never written**; a rerun skips them forever |
| Rate | ≤106 min for 1,430 attempts → **≈4.5 s/chapter (0.22 ch/s)** |
| Second novel (05:30) | 501/501 complete, **≈48 min → 0.17 ch/s** |
| Chapter text | 1,032–2,933 words, but every chapter carries site chrome (“Restore scroll position”, `Translator:`, `Editor:`) |

The bug behind the loss is visible in the code: `completed.add(url)` runs when the
request returns, but the text is held in a dict and appended to the `.md` only
after the whole queue finishes. A crash between those two points marks chapters
complete with no content on disk. `progress.json` is also one global file shared
by every novel.

Current comparison, same title family but one hop instead of three, no proxies:

| Run | Chapters | Time | Rate | Completeness |
| --- | --- | --- | --- | --- |
| `fast_scrape.py` (8 threads, 4 free proxies) | 1,430 attempted | ≤6,376 s | 0.22 ch/s | 1,198 / 1,430 (83.7 %) |
| NF-037 (direct, paced, single hop) | 2,334 | 1,113.8 s | **2.1 ch/s** | **2,334 / 2,334**, zero 1015/429 |

So the remembered run was not fast by proxy magic; it was ~9.5× slower per
chapter than the current direct crawl, and it lost a sixth of the book in a way
the UI could not show.

**Does the proxy math work?** In theory yes, and only one way: rate limits are
per address, so *K* healthy exits each paced at the same 1.2 s give *K×* aggregate
throughput — roughly 8 minutes for this title at *K* = 6. Three facts stop it
being the next lever here:

1. **The pacer is per origin, not per IP.** `Pacer.wait(origin)` keeps one state
   per host, so proxies alone change nothing. Unlocking the *K×* would mean
   forking the module that also owns the per-zone behavioural history, and that
   history is explicitly non-portable across addresses.
2. **The current run is already below what the host blocks.** 2.2 req/s drew
   Cloudflare 1015 after 13 requests; NF-037 sustained 2.1 ch/s with zero blocks.
   Going 6× faster means asking this host for ~6 req/s from six rented identities.
   That is the anti-bot evasion the project explicitly refuses, and when the exits
   die the old code's fallback — 8 unpaced workers on the user's real IP — is the
   exact traffic shape that earns the block.
3. **Free proxies break the trust model.** An exit can read and rewrite every
   chapter, and a proxy's own 200-OK ad or captcha page parses as “content” (both
   scripts fall back to every `<p>` tag). A downloader that advertises verified
   output cannot ship that.

Decision: free proxy rotation is **rejected**, not parked. Optional,
user-supplied proxies stay §6.3 long-term, and would need per-proxy pacing state,
explicit opt-in, and text-level sanity checks (word count, duplicate-title and
chrome detection) before they could be offered without weakening the gate. The
levers that are defensible are already shipped or planned: hop count (NF-036),
index cache reuse, per-host budgets (NF-017), single-archive downloads where a
source offers one, and the lazy read path.

---

## 6. Anti-bot and throughput work plan

### 6.1 Short-term

1. [x] Wire webnovel `throttle` to `--rate-limit` (NF-001 DONE).
2. [x] Hide webnovel `jobs`, which lncrawl cannot honour per crawl (NF-001 DONE).
3. Cache complete indexes.
4. Show phase-aware progress.
5. Add per-host retry/backoff policy.
6. Log request/retry counts.

### 6.2 Medium-term

1. Implement a per-host token bucket with adaptive concurrency.
2. Add platform profiles and adapter tests.
3. Prefer JSON/API endpoints when available.
4. Use connection reuse and HTTP/2.
5. Add conditional requests/caching where possible.
6. Avoid headless browser unless absolutely necessary.
7. Add a “source health” panel with last success, 429 rate and average latency.

### 6.3 Long-term

1. Native Rust fetchers for Tier A sources.
2. A unified cache for metadata, indexes and chapter bodies.
3. Per-source benchmark database.
4. Automatic source health checks.
5. Optional proxy configuration with clear ethical/legal warnings and source-specific rules
   (see §5.7: user-supplied only, per-proxy pacing state, opt-in; free rotation is rejected).

Do not implement aggressive anti-bot evasion. Prefer polite, adaptive, cache-aware fetching and respect source terms and rate limits.

---

## 7. Source support policy

A source is only “supported” when it passes all of:

- [ ] Search returns the correct title (if the source supports search).
- [ ] Book metadata has title and source URL at minimum.
- [ ] Full TOC is fetched, and completeness is proven or explicitly marked unknown.
- [ ] Page count / chapter count consistency check passes.
- [ ] A 1–3 chapter download produces non-empty XHTML.
- [ ] Structured export produces cover, metadata, chapter files and EPUB.
- [ ] Verification passes.
- [ ] A deliberate truncation/failure test fails safely.
- [ ] Source-specific rate limits are documented.
- [ ] A fixture exists for offline regression.

If a source cannot satisfy the index requirement, it can still be offered as **direct URL / best effort**, but it must not claim full completeness.

---

## 8. Test and QA commands

### Rust tests

```bash
cd src-tauri
cargo test
```

### Frontend build

```bash
npm run build
```

### Live index QA

```bash
cd src-tauri
NOVELFORGE_QA_INDEX_URL="https://novelfire.net/book/reverend-insanity" \
NOVELFORGE_QA_TRACE=1 \
cargo run --quiet
```

### Live download QA

```bash
cd src-tauri
NOVELFORGE_QA_QUERY="Reverend Insanity" \
NOVELFORGE_QA_CHAPTERS="1-3" \
cargo run --quiet
```

### Library QA

```bash
cd src-tauri
NOVELFORGE_QA_LIBRARY=1 cargo run --quiet
```

### Custom lncrawl / download root

```bash
NOVELFORGE_QA_LNCRAWL="/path/to/lncrawl" \
NOVELFORGE_QA_DOWNLOAD_DIR="/tmp/novelforge-qa" \
NOVELFORGE_QA_QUERY="https://example.com/book/..." \
NOVELFORGE_QA_CHAPTERS="1-3" \
cargo run --quiet
```

### Source inventory

```bash
LNCRAWL=~/.local/share/novelforge/tools/lncrawl-venv/bin/lncrawl
$LNCRAWL sources list -o json > /tmp/lncrawl-sources.json
$LNCRAWL sources list -s true -o json > /tmp/lncrawl-searchable.json
```

### Metrics to record for every live run

- source URL,
- index pages/requests,
- index time,
- chapter count,
- chapter time,
- retries,
- 429/503 count,
- export time,
- output size,
- verification result.

---

## 9. Execution order

### Phase 0 — Correctness and honesty

- [x] NF-001 webnovel settings (DONE 2026-11-14)
- [x] NF-002 generic index completeness (DONE 2026-11-14)
- [x] NF-005 progress/speed/ETA (DONE 2026-11-14)
- [x] NF-017 anti-bot/throughput foundation (DONE 2026-11-15)

### Phase 1 — Source architecture

- [x] NF-008 parser abstraction (DONE 2026-11-15)
- [x] NF-003 source picker/search coverage (DONE 2026-11-15)
- [x] NF-004 index cache (DONE 2026-11-14)
- [x] NF-009 Korean/copyright availability labels (DONE 2026-11-15)
- [x] NF-018 resolver improvements (DONE 2026-11-16)
- [x] NF-019 NovelFull adapter (DONE 2026-11-15)
- [x] NF-020 Divine Dao Library adapter (DONE 2026-11-15)
- [x] NF-021 NovelPing adapter (DONE 2026-11-15)
- [x] NF-023 NovelFull metadata title/author cleanup (DONE 2026-11-15)
- [x] NF-026 author-page resolution (DONE 2026-11-16)
- [x] NF-027 search/resolve latency (DONE 2026-11-16)

> Open: **NF-024** free public binary release and donation channel.

### Phase 2 — Library lifecycle

- [x] NF-006 duplicate downloads (DONE 2026-11-15)
- [x] NF-007 delete/retry/update (DONE 2026-11-15)
- [x] NF-010 old folder migration (DONE 2026-11-15)
- [x] NF-016 disk cleanup (DONE 2026-11-16)

### Phase 3 — Output quality and polish

- [x] NF-011 human-readable index (DONE 2026-11-15)
- [x] NF-012 chapter cleanup (DONE 2026-11-16)
- [x] NF-013 “All Sites” semantics (DONE 2026-11-16)
- [x] NF-014 Premium label removed (DONE 2026-11-16)
- [x] NF-015 test matrix expansion (DONE 2026-11-16)
- [x] NF-025 frontend interaction tests beyond the Home shelf (DONE 2026-11-16)
- [x] UI pass: responsive shell, readable type scale, backend-focused About (DONE 2026-11-16, pre-NF-024)

### Phase 4 — Manga engine lifecycle

- [x] NF-022 replace archived `kotatsu-dl` with `gallery-dl` (DONE 2026-11-15)

### Phase 5 — Public distribution

- NF-024 free binary release, checksums/licences, Ko-fi support link

### Phase 6 — Books acquisition

- [x] NF-028 book search middle layer and explicit candidate selection (DONE 2026-11-16)
- [x] NF-029 series automation: one-action queue, `Author/Series/NN - Title/`
      layout, skip-complete, per-book not-found reporting (DONE 2026-11-16)
- [x] NF-030 book source evaluation and default policy: measured order,
      budgets, fast-fail, selection guards (DONE 2026-11-16; Ocean explicit-only
      headless)
- [x] NF-031 offline regression fixture for book metadata scoring (DONE
      2026-11-16; 31 tests over real LibGen, Gutenberg and Standard Ebooks
      pages, in `check.sh`)
- [x] NF-032 lawful public-domain sources (Project Gutenberg, Standard Ebooks)
      (DONE 2026-11-16; explicit opt-in only, never part of Auto)

### Phase 7 — Follow-ups from live testing

- [x] NF-033 NovelFire escaped mirror redirect (DONE 2026-11-16; 20/20
      chapters at the shipped throttle, was 15/20)
- [x] NF-034 MangaDex licensed titles are external-only; surface it and point
      elsewhere (DONE 2026-11-16: licensing note names the chapter and publisher
      host instead of `exit 4`; Guardrails rule + QA recipe)
- [x] NF-035 crawl watchdog + one automatic retry (DONE 2026-11-16: 15 min
      pre-chapter / 5 min with chapters, pause-aware; stall resumes, zero-output
      transient restarts, validation failures never retry; live stall+resume
      proves two invocations and a good run still completes in 47 s)
- [x] NF-036 single-hop chapter fetching (DONE 2026-11-16: resolver prefers NovelPhoenix
      over NovelFire on a tie; 20/20 in 56 s vs 342 s, index 42 s vs 194 s)
- [x] NF-037 full 2,334-chapter single-hop run (DONE 2026-11-16: 2,334/2,334 in
      1,113.8 s, ~2.1 ch/s, 0×429/503/1015, 33.7 MB EPUB verified, total 19m 12.6s)
- [x] NF-038 full-length export recompiled a regex per title comparison (5.4 M
      compiles at 2,334 chapters) and stalled at `Exporting 100 %` (DONE
      2026-11-16: export now 16.0 s)
- [x] NF-039 structure chapters during the crawl from lncrawl's per-chapter `.zst`
      store (2026-11-16: implemented; readable + crash-safe before export)
- [x] NF-040 a throttled origin never recovered its pacing interval (DONE
      2026-11-16: `Pacer.learn()` no longer overwrites an already-eased value;
      500-request simulation 5.93 s → 0.50 s; NovelFire had been at 9.8 s/ch)
- [x] NF-041 surface measured source cost and offer a faster-host action on update
      (DONE 2026-11-16: `metadata.json → crawl` records the real run; Library
      cards show measured ch/min and s/chapter; updates inherit the measured
      pace as `expected *`; `Faster source` offers only a resolver-verified,
      measured-cheaper host as a **new** folder. Live: 94.38 ch/min → expected
      2 s; NovelFire rank 5 → NovelPhoenix rank 0)
- [x] NF-042 a fatal crawler error on stdout is reported as "no usable output"
      (DONE 2026-11-16: filtered stdout lines feed `ErrorTail`; bad URL now says
      `Source error · … No crawler found for the domain` in 2.4 s with one
      launch, while the transient stub still retries once)

---

## 10. Open decisions

- [x] Which 3–5 sources should be Tier A after NovelFire/NovelPhoenix?
  - Chosen by the user on 2026-11-14: `https://novelping.com`, `https://novelfull.net`,
    `https://www.divinedaolibrary.com`.
  - Status (2026-11-15): NovelPing 53/53, NovelFull 2,334/2,334 and Divine Dao
    1,229/1,229 have complete indexes. All three now pass the full download
    matrix (3/3 chapters, complete index, verified EPUB) and are `verified`.
    FreeWebNovel (an additional host, not in the original shortlist) also passed
    the full download matrix and is `verified`.
  - Current lncrawl inventory check:
    - `novelfull.net` — enabled, English, `Search: false`.
    - `divinedaolibrary.com` — enabled, English, `Search: false`.
    - `novelping.com` — enabled, English, `Search: false` (added by the
      repository-owned adapter patch; it was absent from the upstream
      inventory).
  - Consequence: search for these sites cannot be delegated to lncrawl's source-search API.
    NF-003/NF-008 must either add native search endpoints/HTML search or offer direct URL
    input only.
- [ ] Should the app use lncrawl for chapter content or native Rust for Tier A sources?
- [ ] What is the minimum acceptable complete-index proof for a generic site?
- [ ] Should incomplete indexes be downloadable as “best effort” with an explicit warning, or blocked entirely?
- [ ] How long should index/metadata cache live?
- [x] Should duplicate downloads update by default or always create a copy?
  - Resolved 2026-11-15: neither. The studio asks explicitly (Update / Keep both /
    Cancel) and the backend fails closed when no policy is supplied.
- [ ] Which Korean-origin English sources matter most to the user?
- [ ] Is browser/headless fallback allowed for sources that require JavaScript?
- [ ] What disk quota should trigger warnings?
- [x] Is “Premium Edition” being kept for design reasons or should it be removed?
  - Resolved 2026-11-16: remove it. The whole sidebar footer status card is gone;
    no tier or build label is shown.

---

## 11. Change log

| Date | Change |
|---|---|
| 2026-11-17 | **Native Windows toolchain bootstrap (NF-057 tool half).** New `scripts/install-tools.ps1` installs the same pinned versions as the bash script — lncrawl `v4.14.0` (editable, all eight patches, scraper patch + `--verify`), gallery-dl `1.32.20`, aget with `click 8.5.1` / `rich 15.0.0` / `playwright 1.65.0` / `nodriver 0.50.99` — under `%LOCALAPPDATA%\novelforge`, per user, no administrator rights, no WSL, re-runnable. It writes `lncrawl.cmd`, `gallery-dl.cmd` and `aget.cmd` into `%LOCALAPPDATA%\novelforge\bin`, so no `PATH` edit and no reboot are needed. `-SkipBrowser` skips the Playwright Chromium download and reports the Anna's Archive/Ocean consequence. Clones use `core.autocrlf=false` because the LF patches will not apply otherwise. Rust side: `tools::installed_bin` searches `dirs::data_local_dir()/novelforge/bin` for `name`, `.exe` and `.cmd`, `install_hint()` is OS-aware, `playwright_chromium_installed` now looks in `dirs::cache_dir()/ms-playwright` with the platform layout (`chrome-win64`/`chrome-win`, `chrome-linux64`/`chrome-linux`), and `process.rs` injects `NOVELFORGE_CHROMIUM` for Books when the environment does not already set it. `scripts/aget.py` no longer assumes `/usr/bin/chromium` off Linux. New unit tests cover the shim lookup and the Chromium layout. `scripts/check-personal-data.sh` now scans `scripts/*.ps1`. README and `docs/RELEASE_READINESS.md` §4.2/§4.4 document the PowerShell path. Gate green: **203** Rust lib + **2** integration + **36** frontend + **38** Python |
| 2026-11-17 | **Windows packaging without a VM (NF-057 packaging half).** New `.github/workflows/windows-build.yml` runs on `windows-latest`: `cargo fmt --check`, `cargo clippy --all-targets -D warnings` (compiles the `MoveFileExW`, `taskkill` and `#[cfg(windows)]` branches), `tauri build --bundles nsis`, SHA-256 manifest, artifact upload, then `cargo test`. The artifact is uploaded **before** the tests so a Windows-only test failure cannot discard the installer. NSIS was chosen over MSI because debloated Windows images often disable the Windows Installer service. `docs/RELEASE_READINESS.md` §4 documents the run steps and the LTSC/Atlas caveats (WebView2 removal, unsigned SmartScreen prompt, expected crawl failures until the native tool bootstrap exists). NF-057 stays OPEN: packaging is now testable, runtime still needs a stock Windows machine. The first run caught one Windows-only clippy lint: `tools::is_executable` returned `false` early and then `true` in the non-Unix branch (`clippy::needless_bool`); the file check now folds into the platform expression |
| 2026-11-17 | **NF-061 DONE (dead-code audit + duplicate-helper dedupe).** Tools: `knip` for TS/TSX, `cargo-machete` for Rust dependencies, `ruff --select F…` and `vulture --min-confidence 80` for Python, `cargo clippy` for Rust, plus a body-similarity scan over Rust and TS. Knip’s first run listed `src/components/home/HeroArt.tsx`, `@testing-library/user-event`, `BYTES_PER_GB`, `formatEta`, `formatPercent`, `statusMeta`, `engineMeta`, `truncateMiddle` and eleven exported types that are only used inside `src/types.ts`; the component, dependency, six helpers and the hero mask were removed, the types were unexported, and the four unreferenced 8 KB `cover-*.jpg` reference assets were deleted. Eight custom classes with no consumer (`btn-outline`, `divider`, `glass-inset`, `hairline`, `ink-rule`, `panel-hover`, `panel-inset`, `panel-strong`) were deleted from `src/styles.css`. Rust: `sources::for_url` (zero references) removed, `SourceProfile::validate` and `sources::all` moved under `#[cfg(test)]`, `ManifestItem.media_type` and its parse line removed, and `core:window:allow-is-maximized` dropped from the capability (the titlebar uses `toggleMaximize`). Duplicate bodies were collapsed: `civil_from_days` now lives in `src-tauri/src/timeutil.rs` (used by `process.rs` and `books.rs`, test moved with it), `metadata::cache_key_hash` reuses `cache::key_hash`, and `duplicates::is_hidden_path` reuses `paths::is_hidden_path`. Frontend duplicates: `hostFromUrl` moved to `src/lib/utils.ts`; the three test copies of `novel()` became `src/test/factories.ts::makeNovel`. Seven unused `scripts/fixtures/novelphoenix/*.html` pages were deleted after a reference scan (surviving fixtures are the ones suites read). `cargo-machete` reports no unused Rust dependencies, `ruff` F-checks pass, `vulture` is clean at 80 % confidence (the 60 % hits are HTMLParser/unittest false positives), `tsc --noUnusedLocals/Parameters` is clean, and all 39 Tauri commands are invoked by the frontend. `knip` is now `npm run knip`, a step in `scripts/check.sh` and a CI step. Gate: **201** Rust lib + **2** integration + **36** frontend + **38** Python; fmt, clippy, knip and the production build are clean |
| 2026-11-17 | **NF-060 DONE (personal-data strip + fixture guard).** New `scripts/check-personal-data.sh` runs first in `scripts/check.sh` and fails on: `/home/<user>/` paths in `src/`, `src-tauri/src`, `scripts/*.sh`, `scripts/*.py`, `package.json` or `tauri.conf.json` (test files excluded); a `bundle.resources` key in `tauri.conf.json`; `include_bytes!`/`include_str!` outside `resources/` before the first `#[cfg(test)]`; and the fixture markers `Reverend Insanity: Chapter 7 \| NovelPhoenix` / `fourth-wing.html` in product code. If `src-tauri/target/release/novelforge(.exe)` exists it is grepped for fixture prose too. Probe: a temporary `src/__guard_probe.ts` containing `/home/<user>/probe/` failed the gate, and removing it passed. Scrub: the three personal paths in `docs/SOURCE_PIPELINE_BACKLOG.md` and every `/home/<user>` value in `docs/measurements/nf-030-2026-11-16.json` were rewritten to `~`, the readiness doc now writes the example as `/home/<user>/...`, `scripts/__pycache__` was deleted and `__pycache__/`/`*.pyc` added to `.gitignore`, and a new `.gitattributes` export-ignores fixtures, `docs/measurements`, the internal backlog and build outputs. The saved-page fixtures stay in the private tree (offline tests need real pages) but cannot reach an artifact. |
| 2026-11-17 | **NF-047 restore verified live; journal now rewrites on boot.** `restore_journal` converted active rows to `Interrupted` in memory but left `jobs.json` saying `running` until shutdown, so a second crash could resurrect a task that was never alive. It now calls `persist_journal()` once after a conversion, and `process::journal_tests::a_running_job_is_restored_as_interrupted` asserts the file contains `interrupted` and no `running`. Live, with temp `XDG_CONFIG_HOME` and a hand-written one-row `running` journal: the relaunched app rewrote the file to `interrupted` before any interaction, and Downloads listed *Reverend Insanity (restore demo)* at 37% with the stored message "NovelForge closed while this download was running. Resume to continue." plus the Resume control. Demo journal and temp config removed afterwards. |
| 2026-11-17 | **NF-045…NF-056 DONE (release-readiness batch).** `fsutil.rs` adds atomic replace (Unix rename, Windows `MoveFileExW`) and every durable file now uses it: `settings.json`, `metadata.json`, `index.json`, `chapters.html`, the retitled EPUB, the index cache, the structured chapter writes and the new `jobs.json`. All JSON is stamped with `schemaVersion`; `Settings`/`ExportMetadata` keep their literal constructors by injecting the key on write. A corrupt bundled catalogue no longer aborts the process. Closing the window no longer silently kills crawls: `CloseRequested`/`ExitRequested` prevent exit while jobs are live and emit `app://quit-requested`; the React modal explains that partial files are kept, and **Stop and quit** calls `confirm_quit`. Jobs now survive a restart: `jobs.json` is written on status change/insert/clear/shutdown, active rows restore as `Interrupted`, and the Downloads row resumes through `resume_interrupted` → the normal verified `retry_download` path. Every log line is appended to `novelforge.log` (UTC, 5 MiB rotation); About → **Copy diagnostics** returns a redacted report (version, platform, Tauri, tool paths, settings, active jobs, last 200 lines), home long paths shortened to `~`. Added a React error boundary, `tauri-plugin-single-instance`, cross-platform process control (`taskkill /PID /T [/F]` on Windows; pause/resume fail closed off Unix), `safe_entry_path` for ZIP asset names, pinned tool versions with `webnovel`/`manga`/`books` install selection, and fmt/clippy gates in `check.sh` + CI. Tests: 201 Rust lib + 2 integration + 36 frontend + 38 Python; clippy clean; production build green. **Still open:** NF-024 (public binary/donation), NF-057 (Windows runtime validation — Wine cannot run WebView2), NF-058 (Android spike), NF-059 (binary-only signing/GPL compliance), NF-060 (strip personal changes). See `docs/RELEASE_READINESS.md`. |
| 2026-11-17 | **NF-044 DONE (visible chapter numbers vs lncrawl serials).** lncrawl's `--chapters` filters `Chapter.serial` (1-based row order), so ORV's visible *Chapter 0* prologue and the TOC's missing `111` made partial downloads one row off. A partial webnovel run now preflights `fetch_chapter_numbers_for` (numbers only, no title passes; the export's index fetch reuses the cache), and `engine_chapter_spec` translates every displayed number through the index row position (`2-3` → serials `3-4`), refusing an incomplete/empty index, a duplicate row or an unlisted number before any folder is created; in-job retries use the same mapping and keep the partial output if the index is gone. `plan_audit` now rejects a numbered, strictly increasing EPUB whose headings do not match the selected visible numbers (`misnumbered_positions` + detail), so a shifted spec fails instead of exporting the neighbour. `full_chapter_spec` builds the refresh spec from the index's real numbers (`0-110,112-551`), `retry_chapter_spec` prefers manifest numbers, `structurer::sweep` maps the crawler's serial to the visible export file (and always keeps exported names), and the picker accepts chapter `0`. Live: ORV `2-3` 2/2 with exactly `0002`/`0003` files and matching EPUB nav, no stray `0004`; ORV `275-277` 3/3 across named/unnamed rows; ORV `111` fails before any folder (`Chapter 111 is not in the source's chapter list`); RI `1-2` 2/2 unchanged. Tests: 195 lib + 2 integration + 35 frontend + 38 Python, `scripts/check.sh` green. |
| 2026-11-17 | **NF-043 DONE (real chapter names for partially-numbered catalogues) + NF-044 logged.** NovelPhoenix leaves 379 of *Reverend Insanity*'s 2,334 rows and 65 of *Omniscient Reader's Viewpoint*'s 551 rows as `Chapter N` even though later rows are named. `fetch_chapter_index_for` now runs a title pass after a complete index and repairs cached entries in place. Donor pass: exact-normalised-title search, `IndexVerified` or better, author agreement when both are known, one hop only, fill **only bare rows** by visible chapter number, ≥95 % coverage, ≥2 names, and a ≥80 % name-agreement guard over shared named rows that rejects shifted donors. Recovery pass: for rows the donor cannot name, read the chapter page and take `Episode N – …`, `Prologue – …`, or a short unlabelled first line (caps 80 pages / 120 s, incremental). Separate `title_lookup_attempted`/`title_recovery_attempted` flags make failures and timeouts resumable. `export_epub` then rewrites the bundled EPUB's `<title>`, `<h1>`, `nav.xhtml` links and `toc.ncx` labels for rows whose name differs (`Chapter 1 - The heart of a demon never has regret even in death`), rebuilding into a temp file, re-auditing, and only then swapping; fully named books keep their original bytes. Live: RI `Filled 379 … from novelfire.net`, chapter 1/2 named and 1168 unchanged; ORV recovered 65 names (chapter 0 → `There are three ways to survive in a ruined world`, 276 → `■■ (1)`, 0 bare); LOTM unchanged with its single bare row recovered. Live RI 1–2 copy export: metadata/index/chapters.html/XHTML names **and** EPUB nav/NCX/headings all carry the borrowed names, `complete=true`, `verified.ok=true`. Tests 190 lib + 2 integration + 32 frontend + 38 Python, `scripts/check.sh` green. **NF-044 logged** (fixed later the same day — see the entry above): ORV starts at visible *Chapter 0* while lncrawl serials start at 1, so `--chapters 0-1` fetched 1 of 2 and failed the audit; partial ORV downloads were off by one. |
| 2026-11-16 | **NF-041 + NF-042 DONE (measured pace, explicit faster host, real crawler reason).** NF-041 persists `CrawlPace` in `metadata.json` (chapters actually crawled this run, never merged totals), exposes `sourceHost`/`sourceRank`/`crawl` on `LibraryNovel`, shows `Measured 94.4 ch/min · 0.64 s/chapter` on the Library card, and annotates updates with `expectedChaptersPerMinute`/`expectedSeconds` (returned **and** emitted; the first implementation returned the pre-annotation clone) plus a `Measured pace · … expected crawl …` dock line and Downloads row. `suggest_faster_source` resolves the stored title and offers only a download-ready host ranked cheaper with relevance ≥ 0.85; the dialog starts a `DuplicateMode::Copy` download and never changes the stored `sourceUrl`. Live: NovelPing 53/53 in 33.7 s → 94.4 ch/min; two deleted chapter files gave `expected 94.38 ch/min · 2 s`, completed 53/53, new pace `2 chapters · 18.41 ch/min`; NovelFire rank 5 → NovelPhoenix rank 0 confidence 1.00; NovelPhoenix → “already fastest”. NF-042 sends filtered stdout error lines to `ErrorTail`, drops traceback frames/INFO rows, treats the final exception as the headline, and keeps `Source error · …` through verification; the real bad-URL run now reports `No crawler found for the domain` in 2.4 s with **1** launch, a stdout-exit-0 stub reports the same with **1**, transient stderr still retries **2**, fatal stderr stays **1**. Tests: 179 lib + 2 integration, 32 frontend, 38 Python; 7 new Rust pace/reason tests plus 3 LibraryPage pace/faster-source tests. |
| 2026-11-16 | **NF-035 DONE (bounded silence + one honest retry).** A crawl that prints nothing, writes nothing and does not change phase is now killed after **900 s** before chapters / **300 s** once chapters flow (15 s poll; paused time subtracted), keeping the work dir. One retry is allowed by failure kind: a stall **resumes** (`--all` → `--resume`, partial output preserved), a non-zero exit with **zero artifacts** and a transient stderr tail **restarts** with identical args, and everything else — bad URL, missing title, incomplete index, cancellation, attempt 2 — fails closed. `ErrorTail::is_transient()` requires a transport/rate-limit marker and lets `unsupported url`/`no crawler`/`404`/`no chapters` win. The UI appends `· no data for mm:ss` after 10 s of silence. `CrawlSpawner` keeps the launch recipe; the pid is retargeted, progress reset and the poller/structurer survive. Tests: 9 (`stall_tests`) including a **real `sleep 30` child killed by the watchdog**, pause accounting, output detection, retry gating, `--resume` args; 172 lib + 2 integration, 29 frontend, 38 Python. **Live:** good NovelPhoenix `1-2` run `Completed` 2/2, 51,071-byte EPUB verified, crawl 44.6 s / total 47.0 s, no false stall; bad URL failed in 2.5 s with one launch and no retry; fault-injected transient stub produced exactly **2** launches and `one automatic retry also failed · … Timeout`; fatal stub produced **1**; stall stub (chapter written, then hang, `NOVELFORGE_QA_STALL_SECS=3`) logged `resuming missing chapters (attempt 2/2)`, **2** launches, `crawl 5.4s · retry 5.4s`. Added **NF-042 OPEN**: lncrawl prints `No crawler found` to stdout and exits 0, so that job loses the reason. |
| 2026-11-16 | **NF-040 DONE (why a NovelFire run crawls at ~6 ch/min).** A live 2,334-chapter *Reverend Insanity* run (novelfire.net URL, `--all --rate-limit 2`) added 7 chapters in 75 s (42→49 files), 12.4 s mean over 19 files, while the same title on NovelPhoenix runs at ~3 ch/s. Cause: lncrawl's persisted per-origin profile had `novelfire.net interval **9.801 s**` (`successes 401`, `failures 6`, first seen 2026-11-10) against `novelphoenix.net 0.333 s` / `freewebnovel.com 1.000 s`. `Session` calls `Pacer.learn(key, profile.interval)` before **every** request and `learn()` assigned unconditionally, erasing the 10 %-per-ten-successes narrowing from `eased()`; the profile's 0.9/0.1 EMA then decayed only ~1 % per ten successes. Simulating the exact loop on the installed `Pacer` from 9.801 s: **9.703 s at 10 requests, 8.864 s at 100, 5.930 s at 500**. Patch: `learn()` now keeps an origin already tracked in `_interval` (this run's fresher measurement) and adopts the profile only for unseen origins; no rate is raised and `eased()` still floors at `--rate-limit`. Same simulation after: **9.703 / 3.975 / 1.386 / 0.514 / 0.500 s** at 10/100/200/300/400 requests. 3 new tests (`scripts/test_lncrawl_scraper_patch.py`, 7/7), applied to the live venv, `--verify` passes, second run no-op. The already-running job keeps its imported module; the recorded profile is still adopted at start of the next run and now eases down. Added **NF-041 OPEN** (surface measured s/ch + ETA and an explicit faster-host action; never auto-switch). |
| 2026-11-16 | **NF-037 + NF-038 DONE (full-length run).** *Reverend Insanity*, 2,334 chapters, novelphoenix.net: completeness **2,334/2,334**, zero empty chapters, **0×429 / 0×503 / 0×1015**, zero retries, 33,718,857-byte EPUB (`verified=true`, `mimetype` first, 2,368 entries, cover present), `metadata.json`/`index.json`/`chapters.html` written, job `Completed`. Clean run: crawl **1,113.8 s (~2.1 ch/s)**, metadata 2.7 s, index 0.0 s (cached), audit 10.1 s, **export 16.0 s**, verify 10.0 s, total **1,152.6 s**. NF-038's quadratic title pass had stalled the same job for >37 min at 100 %; the fix (one `OnceLock` regex, precomputed title maps, map-based archive lookup) makes export real I/O only. The earlier 1.0 ch/s figure came from a run sharing the machine with the NF-038 build/test work, so it is recorded as contaminated, not as the source's rate. |
| 2026-11-16 | **NF-039 implemented**: chapters are structured during the crawl. A `spawn_blocking` poller watches `work_dir/novels/*/chapters/*.zst`, decodes each frame with `ruzstd`, cleans it with the export's `ChapterCleaner`, and writes `chapters/<serial> - <title>.xhtml` atomically; frames caught mid-write are retried when the file changes. The poller is joined before `finalize`, and `export_epub` sweeps only provisional names a real export file replaced (chapters the export could not produce keep theirs, so a partial run cannot delete a crashed crawl's output). Related fix found here: split `.xhtml` files were **not well-formed XML** (`<link …>`, `<br>` emitted unclosed; `xmllint` failed on every exported chapter). Void elements are self-closing now, and provisional chapters get the XHTML namespace plus a `<title>`. 4 new structurer tests on a real saved NovelPhoenix chapter; 1 new cleaner test. **Live (40 chapters, 2026-11-16 15:39 UTC):** files appeared during `DownloadingChapters` (5 → 14 → 22 → 31 → 40 while the `.zst` store grew 7 → 15 → 22 → 31 → 40), the log showed `Live structuring · 1 … / 40 chapter file(s) ready before the final export`, export took **0.4 s**, job `Completed` 40/40, and the final folder had exactly 40 authoritative files + `style.css`, no provisional leftovers, no `.part` files, `xmllint` clean. |
| 2026-11-16 | **Proxy evaluation (no code change).** Reviewed the September scripts in `~/Documents/Scraper/` and their outputs. `fast_scrape.py` rotated 4 surviving free exits across 8 unpaced threads and fell back to 8 direct workers when they died; `tor_scraper.py` capped itself at 0.5 req/s through one Tor exit. Measured from artifacts: *Lord of the Mysteries* took ≤106 min for 1,430 attempts (0.22 ch/s), wrote only 1,198 chapters (234 gaps) while `progress.json` claimed 1,430 — 232 chapters marked done but never persisted. NF-037's direct single-hop run did 2,334/2,334 at **2.1 ch/s**, ~9.5× faster per chapter, zero 1015/429. Free rotation rejected: lncrawl paces per origin, so proxies only help by forking the behavioural pacer and multiplying requests to a host that already blocks at 2.2 req/s; free exits can also rewrite chapter text. Recorded in §5.7; optional user-supplied proxies stay §6.3 long-term. |
| 2026-11-16 | **NF-036 DONE**: the resolver preferred the slower host. `rank_for_host()` used registry position, so NovelFire (3 hops, 7.6 s/chapter) beat NovelPhoenix (1 hop, 0.9–1.4 s) on a confidence tie even though NovelPhoenix was already listed as an alternative. Added an explicit `sources::PREFERENCE` tie-break; UI/search order is unchanged and unknown hosts still sort last. **Live:** resolve "Reverend Insanity" → `novelphoenix.net` (confidence 1.00); fresh index 2,334/2,334 in **42.0 s** (59 pages, 0 retries) vs 194 s; lncrawl TOC + chapters 1–20 **20/20 in 56 s** vs 342 s; app QA 20/20, crawl 54.5 s, total **57.0 s**, 288,310-byte EPUB `verified=true`. Corrected an earlier mis-probe: `.net` uses `/<slug>.html`, not `/novel/<slug>`, so no new adapter was needed. Real `.net`/`.com` fixtures saved under `scripts/fixtures/novelphoenix/`. |
| 2026-11-16 | Speed analysis (no code change). **Concurrency is not the lever.** `runner_concurrency` is the scheduler's job count; the CLI already submits chapters to a 3-thread `TaskManager`, and the scraper's pacer is start-to-start per origin, so extra threads cannot beat one request per gap. The app feels fast because it is lazy (fetch on read, keep bodies on disk), not because it crawls differently. **Hop count is the lever.** A NovelFire chapter is 3 requests (novelfire 302 → readnovel stub → novelphoenix, measured 1.87/0.79/0.85 s). The destination serves the same chapters directly: full 2,334-chapter TOC in **43 s** (24 pages at 1.2 s pacing, 0 blocks), **1.7 s/chapter** including sleeps, **99.6 %** token parity with the NovelFire copy. Projected full run **≈66 min** vs **≈5.0 h**. An unpaced 24-page burst earned Cloudflare 1015 after 13 requests, which the scraper already treats as behavioural backoff. Recorded as **NF-036**; §5.4/§5.5 corrected. |
| 2026-11-16 | NF-033 DONE + crash/log fix. **Crash:** the app job died after 3 m 03.9 s because lncrawl first spent ~90 s timing out on the GitHub source index (5 attempts) and then ~90 s timing out on the FreeWebNovel novel page; exit 1 with 0 files. The source-index sync is now opt-in (`ctx.setup()` defaults to offline; `LNCRAWL_SYNC_REMOTE_INDEX=1` restores it), removing ~90 s and the `Exhausted` loader traceback from every crawl. **NF-033:** the chapter list was always correct; `novelfire.net` intermittently 302s a chapter to `readnovel.site`, whose 2,336-byte “Loading...” stub redirects with JSON-escaped slashes (`https:\/\/novelphoenix.com\/...`). lncrawl-scraper 1.7.0 returned the string verbatim, so `urljoin` built `https://readnovel.site/.../\/\/novelphoenix.com\/...` → permanent 404. `scripts/patch_lncrawl_scraper.py` decodes `\/`, is idempotent, verifies against the saved stub and is wired into `install-tools.sh`. Live: ch 7–8 2/2 in 39 s; ch 1–20 **20/20 in 342 s** (the pre-fix run was 15/20 in 359 s), 299.9 KB EPUB, no mirror 404; app QA ch 7–8 **2/2 in 293 s**, 74,870-byte EPUB `verified=true`. **Logs:** `process.rs` keeps the last 24 stderr lines and appends the headline failure to `Process exited with code N`, so the UI now shows the engine's reason. **Reproducibility:** new repo-owned patches for `--chapters` selection, the offline index, NovelPhoenix `.net`/pagination/search and local source-capability flags; all 8 patches apply cleanly to a pristine v4.14.0 clone in the installer's order, are idempotent, and independently reverse-check. Tests: 4 Python (`test_lncrawl_scraper_patch.py`, real stub fixture) + 2 Rust (`error_tail_*`). Recorded NF-035 (crawl watchdog + one zero-output retry; a rate-limited NovelFire run stalled for 700 s with no output). |
| 2026-11-16 | Live efficiency probe while testing the app. **Reverend Insanity** (2,334 chapters): NovelFire index is complete (24 TOC pages) but 2/10 and 5/20 chapters 404 through malformed `readnovel.site/...\\/\\/novelphoenix.com\\/...` URLs that retry cannot fix; FreeWebNovel served 12/12 and 6/6 with zero failures. Wall-clock: 10 chapters 225 s, 20 chapters 359 s, 12 chapters 241 s, 6 chapters without throttle 149 s — roughly 10 s/chapter at the shipped 2 req/s and 4 s/chapter unthrottled, so ~6.5 h (throttled) to ~2.6 h (unthrottled) for the full novel. **Solo Leveling**: MangaDex English aggregate is down to 16 chapters (licensed, external-only), Weebcentral has 201 chapters; measured 18 images/8.6 MB in 9.2 s, 67 images/33 MB in 30.7 s → ~10 s and ~11 MB per chapter, ~34 min and ~2.2 GB for the series. Recorded NF-033 (NovelFire URL corruption) and NF-034 (licensed MangaDex UX). |
| 2026-11-16 | NF-032 DONE: lawful public-domain sources as an explicit opt-in. `scripts/aget.py` adds `PUBLIC_DOMAIN_SOURCES = ("gutenberg", "standardebooks")` + `ALL_SOURCES` while `resolve_sources("auto")` still returns the user-approved `libgen → welib → annas → ocean`; new `gutenberg_download` (Gutenberg HTML search → book page confirms `dcterms:language` → `.epub3.images`) and `standardebooks_download` (HTML search → plain EPUB; the plain `/downloads/<slug>.epub` URL is an XHTML interstitial, so `?source=download` is appended). Pure parsers `parse_gutenberg_search`/`parse_gutenberg_book`/`parse_standardebooks_search`, `language_name`, `is_english` and `payload_matches_format`/`save_verified_payload` reject non-EPUB/PDF bytes before writing. Rust allow-list, `BookSource`, Books source cards (opt-in note + “Auto order” hint) and a Guardrails rule updated; new `book_source_accepts_public_domain_sources` and BooksPage opt-in test. Real fixtures for both catalogues; Python suite 31 tests. Live: Gutenberg *Pride and Prejudice* 24,864,480 bytes in 96.0 s, *The Time Machine* app E2E 307,155 bytes in 4.5 s, Standard Ebooks *Pride and Prejudice* 831,932 bytes in 4.6 s, series *Barchester Towers* shelved under `Anthony Trollope/The Chronicles of Barsetshire/02 - Barchester Towers/`, and both sources fail closed on *Dune*. Gates: 31 Python tests, `npm test` 29/29, `cargo test --lib` 151/151, `npx tsc --noEmit`, production build, `bash scripts/check.sh` passing. |
| 2026-11-14 | Created this backlog after live NovelFire testing, E2E retry testing, and lncrawl source inventory audit. |
| 2026-11-14 | NF-001 DONE: webnovel throttle now passes `--rate-limit 2`; retries use the same limiter; misleading jobs slider removed for webnovels; tests added. |
| 2026-11-14 | NF-002 DONE: generic TOC walker now follows query/path/next-link/AJAX pagination, reads JSON-LD ItemLists and templates, and requires a completeness proof. Incomplete indexes fail the job and write `indexDetail`. Live NovelPing 53/53 and Divine Dao 1,227/1,227; NovelFull live check blocked by Cloudflare 403 (deferred to NF-017). |
| 2026-11-14 | NF-005 DONE: each webnovel job uses an isolated `LNCRAWL_DATA_PATH`; progress measures the crawl work dir then the destination, with phase tracking, real chapter-file fallback, 8 s speed and 20 s ETA windows, pause-time exclusion, idle time, and a per-stage `Timing · …` line. Successful runs remove the work tree; library scans ignore it. Live NovelPhoenix 113-chapter index and chapters 1–8 completed with pause/resume; final timing `crawl 42.5s · index 5.3s · total 48.0s`. Tests 45/45; `npm run build` passing. |
| 2026-11-14 | NF-004 DONE: complete chapter indexes are cached on disk for 6 h keyed by source host plus canonical TOC URL, with temp-file writes, key re-checks, TTL expiry and incomplete-index rejection. The picker and download pipeline share the cache; the UI shows cache age and a Refresh button that bypasses it. Live evidence: fresh 3.7 s vs cached 0.0 s, warm download `index 0.0s`, stale TTL and manual refresh both refetched. Tests 49/49; `npm run build` passing. |
| 2026-11-14 | User selected `novelping.com`, `novelfull.net`, `divinedaolibrary.com` as candidate Tier A sources for NF-003/NF-008. |
| 2026-11-15 | NF-006 DONE: duplicate identity is the normalized source URL (`normalize_source_url`), with a library scan that picks the newest complete match and reports `matches`. Downloads fail closed without an explicit Update/Copy policy; the studio prompts with Update / Keep both / Cancel. Update merges the previous manifest, preserves chapters outside the current run, removes superseded files and names chapter files by serial; Copy allocates `slug-copy-N`. Live NovelPhoenix evidence: repeat blocked with one folder, update 3–4 kept 1–2 (`4/113`), copy created `mother-of-learning-copy-2`. Fixed the empty `normalize_title("Chapter 1")` collision found in live QA. Tests 54/54; `npm run build` passing. |
| 2026-11-15 | NF-007 DONE: library actions operate on the exact clicked folder through `DownloadOptions.targetFolder`. New `library_actions.rs` adds details, chapters, delete, retry and update; `paths.rs` enforces canonical containment and hidden-path rules. Retry stores and respects the original chapter selection, so a failed chapter in a 3–4 run never expands to the whole novel; update fetches a fresh complete index and selects new chapters (or `1-N` on refresh); delete refuses the root, outside paths, hidden trees, metadata-less folders and running jobs. Studio card actions, confirmation dialogs and an index viewer added. Live evidence: retry downloaded only 3–4 and preserved 1–2; selection-limited retry downloaded only chapter 4; update downloaded only 112–113 and preserved 1–111 (`113/113`); up-to-date and all delete guards refused correctly. Tests 62/62; `npm run build` passing. |
| 2026-11-15 | NF-017 DONE: shared `transport.rs` adds per-host source profiles, configurable concurrency/gap/chapter-rate overrides, a gate that starts at tested safe concurrency and ramps only after 20 clean responses, immediate 429/503 backoff with `Retry-After` and jitter, Cloudflare 403 detection, a curl fallback for TLS-fingerprint-blocked hosts, and a blocked-host cooldown. Metrics appear in job logs, the chapter-index panel, QA output and Guardrails; `lncrawl --rate-limit` now reads the source profile. Live NovelFull index 2,334/2,334 in 67.7 s over 59 curl TOC pages at 1/1 concurrency; cached repeat 0.0 s; simulated 429 and Cloudflare probes showed one retry then a block with no failure loop; a 1,234 ms / 1.5 req/s per-host override was verified live. Tests 74/74; `npm run build` passing. |
| 2026-11-15 | NF-008 DONE: `sources.rs` declarative registry (profiles, support levels, parser kind, capabilities, offline validation), metadata/resolver dispatch switched from host string checks to the registry, `list_source_profiles` command and a Guardrails “Source support” table. FreeWebNovel promoted to `verified`: 2,334/2,334 chapters from 59 TOC pages in 49 s, 1–3 download 3/3 with cover/metadata/EPUB, truncated fixture rejected (`3 of 10 advertised`). Generic metadata now reads JSON-LD/`og:author`/`og:novel:author`, fixing the live `author: null`. Webnovel.com stays `metadata-only`: reqwest gets Cloudflare 403 but the curl fallback resolves metadata, the catalog lists 877 rows but only 832 links (45 locked), and a live run failed the index gate after downloading a verified chapter. Tests 82/82; `npm run build` passing. |
| 2026-11-15 | NF-003 DONE: added a reproducible 338-source lncrawl catalogue (`scripts/build_source_catalogue.py` + `catalogue.rs`), a grouped source picker with per-language groups and a direct-only group, dynamic result filtering, and per-source failure reporting. `search_webnovels` accepts an explicit `sources` list; empty means the NovelForge registry, `all` means every searchable non-manga source, and a direct-only host returns an explicit failure. Live evidence: WonderNovels 1 result/8.1 s, registry default 13 results/4.1 s, `all` 43 sources/47 results/0 failures/76.2 s, NovelFull direct-only failure. Tests 87/87; `npm run build` passing. |
| 2026-11-15 | Logged follow-ups: NF-019 NovelFull stale chapter adapter, NF-020 Divine Dao Library redesign, NF-021 NovelPing missing adapter, NF-022 archived kotatsu-dl manga engine. NF-022 records the gallery-dl evaluation: active v1.32.20, 387 sites, per-chapter CBZ/directory output, 5–6 file adapter swap; recommendation is gallery-dl over Suwayomi/Tachidesk, pending user approval. |
| 2026-11-15 | NF-022 DONE: replaced archived `kotatsu-dl` with `gallery-dl` v1.32.20. Backend now resolves/probes gallery-dl, uses a private work tree, `--child-filter` for exact chapter numbers, `--write-info-json` as the metadata source, and `package_manga_directory` to build one CBZ/ZIP with chapter folders (deduping alternate uploads of the same language/volume/chapter). UUID titles are replaced with the extractor’s series title; settings migrate `kotatsuPath` → `galleryDlPath` and drop Java/parallel jobs. `scripts/install-tools.sh`, README and the Tauri bundle description now install and describe gallery-dl. Live catalogue **26 manga extractors**; MangaDex `chapters=1` produced 4 volume chapters/27 pages, a 27.1 MB `Oshimai.cbz`, real metadata and cover, verified folder `oshimai-<job>` in 24.6 s. Tests 93/93; `npm run build` passing. |
| 2026-11-15 | NF-019 DONE: patched the lncrawl NovelFull adapter for the AJAX catalogue (`#truyen-id`, `#total-page`, `/ajax-chapter-list`), with fail-closed short/empty-page checks and a legacy-template fallback. Patch is repository-owned (`scripts/patches/lncrawl-v4.14.0-novelfull-ajax.patch`, `scripts/apply-lncrawl-patches.sh`); `install-tools.sh` now source-installs lncrawl v4.14.0 so the patch survives reinstall. Live 59-page crawl enumerated 2,334 chapters; QA downloaded 1–3 in 2m 03.9s with 3/3 audit, 2,334-chapter `indexComplete=true`, cover, chapter XHTML and a 61,523-byte verified EPUB. NovelFull promoted to `verified`. Logged NF-023 for the remaining title suffix/`author: null` metadata gap. Tests 93/93; `npm run build` passing. |
| 2026-11-15 | NF-020 DONE: rewrote the lncrawl Divine Dao Library adapter for the Fictioneer redesign (`h1.story__identity-title`, `.story__identity-meta a.author`, `og:image`, `.story__summary`, `a.chapter-group__list-item-link`, `#chapter-content p`). Added `scripts/patches/lncrawl-v4.14.0-divinedaolibrary-redesign.patch`; the patch applier now handles both source patches. Live story page enumerated 1,229 chapters; QA downloaded 1–3 in 12.3s with 3/3 audit, 1,229/1,229 `indexComplete=true`, clean title/author, 400×600 cover, chapter XHTML and an 86,212-byte verified EPUB. Divine Dao Library promoted to `verified`. Tests 93/93; `npm run build` passing. |
| 2026-11-15 | NF-021 DONE: added the missing lncrawl NovelPing adapter (`#rating[data-novel-id]` → `/ajax/chapter-archive`, parsing the `template[data-chapter-item-template]` list, `og:novel:*` metadata, `#chr-content p` body, fail-closed empty archive). Added `scripts/patches/lncrawl-v4.14.0-novelping-adapter.patch`; the patch applier now handles all three source patches. Regenerated `lncrawl-sources.json` (338 → 339 enabled sources, NovelPing is direct-URL only). Live archive enumerated 53 chapters; QA downloaded 1–3 in 9.5s with 3/3 audit, 53/53 `indexComplete=true`, cover, chapter XHTML and a 50,960-byte verified EPUB. NovelPing promoted to `verified`. Tests 94/94; `npm run build` passing. |
| 2026-11-15 | NF-023 DONE: added `ParserKind::NovelFull` and a NovelFull metadata parser that reads `h1.tit`, `/author/` links (both names), `/genre/` tags, `.m-desc > div.txt` and `#list-chapter[data-total-chapters]`, with a suffix-stripping `og:title` fallback. Added `scripts/patches/lncrawl-v4.14.0-novelfull-metadata.patch` so the EPUB itself gets a clean `dc:title` and a populated `dc:creator`; the applier now runs four patches. `export_epub` prefers the primary `metadata.author` while keeping the full `authors` list. Live 1–3 QA: `title="Reverend Insanity"`, `author="Gu Zhen Ren"`, `authors=["Gu Zhen Ren","蛊真人"]`, 2,334/2,334 index, 3/3 audit, 61,542-byte verified EPUB in 53.9s. Tests 96/96; `npm run build` passing. |
| 2026-11-15 | NF-009 DONE: added evidence-backed availability metadata instead of a universal Korean promise. `SourceProfile`/`SourceSummary` gained `availability_note`; NovelFire's origins are now `zh, ja` (Korean removed) with a copyright/catalogue note, shown in Guardrails. `CatalogueSource` gained `originLanguages`, `translatedFromKorean` (English site + registry `ko`) and `availabilityNote`. Search UI adds an origin filter (Any / Korean / Chinese / Japanese, best effort), site-language and Korean-origin chips, per-source notes, picker Korean markers, and empty-state copy naming the sources that answered and explaining that catalogues differ. README and backlog document the limitation. Live `I Alone Level-Up` search returned 12 results from 2 sources in 3.9s: NovelPhoenix found the title, NovelFire did not. Tests 98/98; `npm run build` passing. |
| 2026-11-15 | NF-010 DONE: loose archive folders can be adopted in place. `list_library` returns `archives` with an explicit adoptability reason; `export::adopt_epub` audits the EPUB, writes chapters/cover/metadata beside it without touching the original, and is idempotent. Adopted entries carry `provenance: "migrated"`, no source URL and no retry/update actions. The Library page adds a migration queue, an **Adopt** button, a **Migrated** badge and summary tiles for novels/adopted/loose/size; the index dialog labels a local EPUB index instead of the missing-remote-index warning. Live old folder: original EPUB SHA-256 unchanged, 1/1 chapter, 2 novels + 0 archives after adoption; a second adoption left `metadata.json` byte-identical. Live UI one-click adoption produced a migrated card and toast; temporary demo folder removed. Tests 101/101; `npm run build` passing. |
| 2026-11-15 | NF-011 DONE: every structured folder now gets `index.json` (compact chapter list with number, title, URL, volume, downloaded flag and local file path) and `chapters.html` (self-contained searchable page with source link, downloaded/missing counts, text filter, “Missing only” toggle and **Not downloaded** markers). Written by `export_epub`/`export_cbz` after `merge_previous_export`, and backfilled by `library_novel` for older folders. The lncrawl EPUB is left byte-identical; the index is a sidecar. Fresh NovelPing 1–2 run wrote 13,211-byte JSON and 15,849-byte HTML (2 downloaded / 51 missing) alongside a verified EPUB; the real 2,334-chapter folder wrote 557,837-byte JSON and 591,063-byte HTML with 3/2,331 rows; the adopted 1-chapter folder wrote its migrated index. Tests 103/103; `npm run build` passing. |
| 2026-11-16 | NF-012 DONE: added `chapter_clean.rs`, a conservative XHTML cleaner wired into `export_epub` before chapter files are written. It removes `aria-hidden`/numeric title markers, duplicate leading headings, title-only leading paragraphs (including NovelFull’s bold `Chapter 1 – …` subtitle when a heading already exists), and nav/ad/donation/sharing chrome. Selectors are configurable through the new `SourceProfile::chapter_chrome` registry field; `#chapter`, `#content`, `html`, `head`, `body` and `main` are protected. Unparseable input keeps the original bytes. Real exported public-domain fixture plus synthetic fixtures added; live NovelFull 1–2 exported 86 of 87 paragraphs (only the duplicate subtitle removed) with zero `aria-hidden`/`<strong>Chapter` markers and a verified 50,668-byte EPUB. Tests 113/113; `npm run build` passing. |
| 2026-11-16 | NF-013 DONE: the Home source filter is now labelled from the downloaded shelf instead of “All Sites”. The first option is `All downloaded sources · N novels`; each host is `host · N novels` (`Local / imported · N novels` when no source URL exists), and counts are built from the same shelf rows that are displayed. The select exposes the full label through `title`/`aria-label`, the field is wide enough for the full label at normal sizes, and a vanished source falls back to “all” instead of a blank filter. Live app with the real 2-novel library showed `All downloaded sources · 2 novels`, `Local / imported · 1 novel` and `novelfire.net · 1 novel`; selecting each source filtered to the matching single novel. `npm run build` passing. |
| 2026-11-16 | NF-014 DONE: removed the sidebar footer status card entirely. “Premium Edition”, its crown glyph, the hardcoded `v1.0` and the engine-count line are gone, and the unused `version`/`enginesReady`/`enginesTotal` props were dropped from `Sidebar` and `App`. The sidebar now ends at the About navigation item. About already showed the real `v0.1.0` build version and made no paid-tier claim, so no copy needed deleting. `grep -rn "Premium" src` returns nothing; live desktop check shows an empty footer and a normal About page. `npm run build` passing. |
| 2026-11-16 | Logged NF-024: free public binary release (installers/portable builds, SHA-256, third-party licences, mirror) plus a voluntary Ko-fi link. No store submission and no paywall; details pending approval. |
| 2026-11-16 | Logged NF-025: frontend interaction tests for search hydration/failures, the incomplete-index warning, Library actions against the exact clicked folder, and persisted-task hydration. NF-015 covers the offline export path and the Home source filter; the remaining UI paths are still manual-only. |
| 2026-11-16 | NF-015 DONE: expanded the test matrix to three offline layers plus an opt-in live matrix. New `src-tauri/tests/offline_export.rs` (2 tests) builds a real fixture EPUB and drives `audit_epub` + `export_epub`, asserting cleaned chapters, cover, `metadata.json`, `index.json`, `chapters.html`, the indexed/downloaded split, and that an incomplete index forces `complete: false` while keeping partial files; `export`/`model` became `pub` for it. New Vitest setup (`vitest.config.ts`, `src/test/setup.ts`, `npm test`) and `src/pages/HomePage.test.tsx` (3 tests) cover the NF-013 source-filter labels, counts, filtering and vanished-source fallback. `scripts/check.sh` runs Rust unit + integration, frontend tests and the production build; `.github/workflows/ci.yml` mirrors it. `scripts/source-smoke.sh` refuses to run live without `NOVELFORGE_ALLOW_LIVE=1` and covers NovelPhoenix, NovelFull, FreeWebNovel, Divine Dao, NovelPing and NovelFire. QA download output now ends with a `transport report` (requests/retries/429/503/curl/blocked/avg/concurrency/gap/rate) next to the existing `Timing` line. `docs/QA_MATRIX.md` records the P0 regression map, live matrix, manual cancellation check and the four-part new-source merge gate; `sources::tests::every_profile_documents_evidence` enforces registry evidence. Live re-runs: NovelPhoenix `113 chapters from 3 TOC pages`, 2/2 audit, 119,453-byte EPUB, 17.8s, `0×429 · 0 blocked` (`/tmp/opencode/nf015-smoke-20261116-023914/novelphoenix.log`); NovelFull 2/2 audit, verified EPUB, `1 curl · 0 blocked` (`/tmp/opencode/nf015-smoke-20261116-024114/novelfull.log`). Totals: Rust 114 unit + 2 integration, frontend 3/3, `npm run build` passing. |
| 2026-11-16 | NF-016 DONE: disk accounting and cleanup policy. New `storage.rs` measures library/partial/work trees, enforces an optional library quota (GB setting, `0` unlimited, 100 TiB clamp) before the resolver runs for brand-new folders only, removes abandoned `.novelforge-work/<job id>` trees while keeping live jobs, and deletes incomplete structured folders only (never complete or busy ones). `LibraryNovel.diskBytes` and a `LibraryResponse.storage` summary are surfaced by `list_library` after the index backfill; successful imports remove their work tree when the new `cleanupWorkAfterImport` setting is on. UI: Library **Disk usage** panel with per-novel sizes and two confirmation dialogs, Settings quota + current size + cleanup toggle, Webnovels over-quota banner and a 300+ chapter all-chapters estimate prompt. QA probes: `NOVELFORGE_QA_STORAGE=report|work|partials|all`, `NOVELFORGE_QA_QUOTA_BYTES`, `NOVELFORGE_QA_KEEP_WORK`. Evidence: fixture `library=15586 partial=7265 work=4098`, work cleanup freed 4,098 B, partial cleanup freed 7,265 B with the complete folder intact, 1-byte quota failed before network, live NovelPhoenix 1–2 at 100 MB quota succeeded and removed its work dir (`library=306485`), live NovelFull with cleanup off kept 1,906,959 B and reclaimed it via the Library action. Desktop check showed the Disk usage panel (`1.8 MB`, per-novel `62.1 KB`) and the Settings quota field. Tests: Rust 119 unit + 2 integration, frontend 8/8, `npm run build` passing. |
| 2026-11-16 | NF-018 DONE: resolver now returns confidence, match type, consulted sources and alternatives. Added author mode (toggle or `author:`/`by:`), author verification on candidate detail pages, source-native alias matching from JSON-LD `alternateName` and “Alternative names” rows, AniList `native` titles for Hangul/CJK queries, diacritic/full-width Unicode folding, and no-match errors naming every consulted source (including AniList/Wikipedia). Candidate inspection is round-robin across sources and ties break on search rank, fixing NovelFull noise hiding NovelPhoenix hits. `process::start_download` refuses non-URL webnovel inputs below `0.85` before creating a folder; the studio shows a review warning and clickable alternatives instead. Live: title `1.00`, `master of gu` → Reverend Insanity `1.00`, author `Gu Zhen Ren` → Reverend Insanity `1.00 author`, no-match listed all four sources, and `reverend insanity 2` at `78%` was refused with no folder. Desktop checks confirmed the author panel, the 78% review warning and alternative switching. Tests: Rust 122 unit + 2 integration, frontend 11/11, `npm run build` passing. |
| 2026-11-16 | NF-026 DONE: the resolver now reads source author pages instead of trusting title search for author queries. `SourceProfile.author_page` + `AuthorSlug` (NovelFire `gu-zhen-ren`, NovelPhoenix `Gu+Zhen+Ren`); `metadata::fetch_author_page` prefers JSON-LD `ItemList`, then NovelFire's first `ul.novel-list` (recommendation lists excluded), then `h3.tit`/titled anchors, and takes the author name from the breadcrumb, `h1` or `CollectionPage`; a canonical-path + label guard rejects NovelFire's home-page answer for unknown slugs. Author mode answers from pages and only searches sources without one; auto mode runs pages alongside search and answers exact author names directly; bibliography keeps shelf order and prefers a download-ready URL per title. Live: author `Gu Zhen Ren` 4 novels (Reverend Insanity + Infinite Bloodcore, Mysteries of Immortal Puppet Master, Advent of Immortal Truth) in 1.6–2.1 s (was 2 novels / 12.0 s); NovelFire-only 4 novels 1.5 s (was no match / 13.8 s); auto 4.8–5.5 s (was 12.6 s); NovelPhoenix-only still 2 novels. Desktop Author panel showed “100% author match” plus the three other-source matches. Tests: Rust 129 unit + 2 integration, frontend 11/11, `npm run build` passing. |
| 2026-11-16 | Logged NF-027: resolve latency. Measured `lncrawl search` at 5.8 s for one source, 12.0 s author resolve before NF-026, `master of gu` at 17.1 s (AniList + Wikipedia + second search), unknown author at 15.6 s. Causes: per-source Python startup, 8 candidate detail fetches at 3 concurrency with 12 s timeouts, no search/summary cache, second search after canonical-title lookup. Planned: TTL result cache, batch verification (6 concurrency / 8 s timeout / early exit), one crawler invocation for all sources, skip the duplicate search, and treat an empty author page as authoritative. |
| 2026-11-16 | NF-025 DONE: frontend interaction coverage now extends past the Home shelf. New `NovelSearchPanel.test.tsx` (5 tests, mocked `@/lib/tauri`) covers a partial response with a failed-source list and metadata-failure fallback, an all-source failure list, a zero-hit response that must not invent rows, the two-character guard and the direct-URL-only source guard. New `LibraryPage.test.tsx` (6 tests, stubbed manager) covers the incomplete summary and **Migrated** badge, hides retry/update for migrated entries, opens the incomplete-index warning with its `2,140 of 2,334 chapters listed` detail, shows the migrated `local EPUB index` note without the remote warning, and asserts Retry/Update/Delete/Adopt against the exact clicked folder or archive path once. New `useDownloadManager.test.tsx` boots the real hook and `DownloadsPage` from a persisted two-task `api.getTasks` response and asserts stored `paused`/`completed` statuses plus 42%/100% progress with no fabricated animation. `npm test` is now 6 files / 23 tests (was 3/11); `scripts/check.sh` passes with Rust 129 unit + 2 integration, frontend 23/23 and `npm run build`. |
| 2026-11-16 | NF-027 DONE: resolver latency. New `cache.rs` on-disk TTL cache (key stored in each entry, atomic temp write, 15-min TTL, env-overridable) caches successful per-source search batches, author pages and detail summaries; `search_targets` and `collect_author_pages` read it before spawning work. Candidate verification runs a three-page first wave and stops immediately on an exact author (`>= 0.95`), otherwise continues at concurrency 6 with an 8 s summary timeout. Empty author pages are authoritative when the canonical path contains `/author` and the label matches; a second crawler search is skipped when the canonical title equals the input or is already in the first rows. `SearchResponse`/`ResolvedTitle` gained `cached` + `cacheAgeSeconds`; the results header and resolved panel label cache age, and the QA harness prints `cached`/`cache_age`/`elapsed` for search and resolve (including failures). Multi-source crawler reuse was investigated and rejected: the lncrawl CLI accepts one source per `-s`, repeated flags keep only the last, and comma/space lists fail. Live (debug, `/tmp/opencode/nf027-cache`): author `Gu Zhen Ren` cold 1.72 s / warm 0.00 s (`cached=true`, age 5 s); title `reverend insanity` cold 5.17 s / warm 0.74 s (`cached=true`, age 8 s); unknown author cold 1.82 s / warm 0.84 s (was 15.6 s); search `Gu Zhen Ren` warm 0.0 s (`cached=true`, age 4 s); auto `master of gu` cold 14.27 s / warm 1.68 s (was 17.1 s). Tests: Rust 136 unit + 2 integration, frontend 24/24, `npm run build` passing. |
| 2026-11-16 | UI pass (pre-NF-024): responsive shell, readable type scale and a backend-focused About page. The sidebar collapses to a 74 px icon rail below `lg` (labels become tooltips/`aria-label`s and counts move to corner badges); the right rail is 340 px and widens to 380 px on `2xl`; main content uses `px-4 / sm:px-6 / xl:px-8` inside a 1680 px column, the Home hero is full-bleed, the Library table scrolls horizontally with a 560 px floor, and the Downloads/Guardrails/Settings grids collapse to one column; the Tauri minimum window is now 900×620 (was 1180×720). Type scale: all 21 `text-[Npx]` users were remapped upward (7.5→9.5, 8→10, 9→11, 10→12, 11→13, 12→14, 13→15) and shared component sizes were raised (field 14.5 px, buttons 12.5/11.5 px, chips 10 px / 22 px, section labels 10.5 px). About now explains the Rust/Tokio pipeline (resolve → fetch → verify → archive), lists what the backend owns (registry, transport policy, TTL caches, fail-closed verification), and no longer describes itself as a wrapper. Verified at 1440×900 and at the 900×620 minimum on Home, About, Guardrails, Library and Books: no clipped text, no overlapping columns, clear sidebar/content gap. `scripts/check.sh` passes: Rust 136 unit + 2 integration, frontend 24/24, production build. |
| 2026-11-16 | Logged NF-028/NF-029/NF-030 after a books-path review. The current `aget` helper takes the first EPUB-looking row from WeLib, Anna's Archive or OceanOfPDF with no title/author/ISBN check, so `fourth wing` downloaded the wrong book. Live probes: WeLib `.st` returns `HTTP 403` in under 1 s; Anna's Archive was still printing `searching…` when killed at 95 s (16 × 4 s DDoS-Guard poll, then a 14 × 4 s slow-page poll); OceanOfPDF's code launches `nodriver` with `headless=False` and an offscreen window, which surfaced a visible Chromium on Hyprland. NF-028 proposes a catalogue-first search (`search_books` with Hardcover/Open Library candidates), explicit user selection, metadata-scored file rows and background-only fetching. NF-029 proposes series resolution (Hardcover `series.book_series` with position ordering/dedupe, Open Library `series:"…"` fallback) and one-action EPUB queues under `Author/Series/NN - Title.epub`. NF-030 records the source comparison (Anna's membership API, LibGen, WeLib, Ocean, optional Shelfmark/stacks) and a measured default order; Ocean becomes manual-only. No app code changed in this cycle. |
| 2026-11-16 | NF-028 DONE: catalogue-first book search, series resolution and verified free-source download. Backend `books.rs` adds `search_books` (Hardcover GraphQL with a local token, Open Library fallback) with a 1 h `books::v7` cache; Open Library series resolution merges title + `series:"…"` searches, verifies membership per work's editions, reads real positions (`#1`, `#3.5`, `, Book 2`, `-- 2`), drops omnibus/companion rows and retries 429/5xx five-at-a-time. `scripts/aget.py` adds a LibGen HTML source with title/author/language/ISBN scoring, `MATCH_THRESHOLD = 0.62` and fail-closed `choose_candidate` (no `rows[0]` fallback); Ocean is headless and title-verified per candidate, Anna's waits are bounded. `DownloadOptions` carries title/author/ISBN through `process.rs`; commands are registered and the Books UI has catalogue results, source cards, a series checklist and per-book Fetch/queue with no download before selection; Settings stores the optional Hardcover token locally (0600). Live: `The Empyrean` → Fourth Wing #1 / Iron Flame #2 / Onyx Storm #3 in 8.07 s; `The Lord of the Rings` → 1/2/3 plus The Hobbit in 14.93 s; *Fourth Wing* (Yarros, `9781649374042`) downloaded from LibGen as a 5,064,906-byte EPUB with `metadata.json`, `index.json`, `chapters.html`, cover, 46 chapter XHTML, `Images/` and `Styles/`; a wrong-expectation probe wrote nothing and reported near misses. Added NF-031 for an offline scoring fixture. Gates: `npx tsc --noEmit`, `npm test` 27/27, `cargo test --lib` 148/148, `scripts/check.sh` green. |
| 2026-11-16 | NF-029 DONE: series automation. `DownloadOptions` gains `bookSeries`/`bookSeriesPosition`; `process.rs` adds `safe_segment`, `series_position_label` (`1` → `01`, `3.5` → `03.5`), `series_book_stem` and `book_series_destination`, so a books job with a catalogue series shelves into `<root>/<Author>/<Series>/<NN> - <Title>/`, renames the EPUB to `<NN> - <Title>.epub` before audit/export, and keeps the non-series `<slug>-<id8>` behaviour unchanged. A complete destination folder is refused with `“<title>” is already complete in the library at …`; a Books failure with no EPUB now reports `Book not found: no catalogue-verified EPUB was produced…`. `qa.rs` reads `NOVELFORGE_QA_BOOK_SERIES`/`_POSITION`. The Books UI adds `shelvesBook`/`isShelved` (complete + exact normalised title + compatible author), removes shelved books from `queueSeries`, shows `already shelved`/`shelved`, and one button queues every selected released book. Live: *Fourth Wing* → `…/Rebecca Yarros/The Empyrean/01 - Fourth Wing/01 - Fourth Wing.epub` 5,064,906 bytes + sidecars + 46 chapters; repeat run refused as already complete; library scan sees the nested novel as complete (17,738,903 bytes); all-mirror-timeout run reported not found. Tests: `series_layout_uses_author_series_and_padded_position`, `series_layout_falls_back_without_a_position_and_stays_contained`, `BooksPage.test.tsx` skip-complete. User approved `libgen → welib → annas → ocean` and parked NF-024; recorded in NF-030. Gates: `npx tsc --noEmit`, `npm test` 28/28, `cargo test --lib` 150/150, `scripts/check.sh` green. |
| 2026-11-16 | NF-030 DONE: book source policy measured and enforced. `scripts/aget.py` now bounds every source with one per-source budget (`SOURCE_BUDGET_SECONDS`, `--budget`, `NOVELFORGE_SOURCE_BUDGET`, default 150 s) and checks the deadline inside every polling loop, so a DDoS-Guard/Cloudflare hold ends in one `… after N.Ns — skipped` line instead of a hung job (Anna's download wait was 10 × 180 s before this). LibGen keeps 25 s for the transfer, caps each mirror search at 15 s and downloads from up to three verified copies because one stalled storage node used to consume the whole budget; WeLib checks both mirrors and reports `HTTP 403`/`CAPTCHA` in ~3 s; Anna's filters irrelevant rows by query tokens and renormalises title-only rows only at title ≥ 0.9; Ocean cleans `[PDF] [EPUB] … Download` headings and clears a stale Chromium profile lock. Selection guards: 0.62 threshold plus a 0.55 title floor (`Dune Messiah` was 0.72 on author alone), `PREFIX_AUTHOR_FLOOR 0.8` for punctuation-suffix titles (`Dune: Tales of Dune Expanded Edition`), omnibus/collection rejection, author score divided by the row's names (`Frank Herbert; Brian Herbert & Kevin J Anderson` scored 1.0), and a declared non-English row rejected when no `--lang` was requested (`Dune 2a ed`, Spanish, was downloaded before this rule). New `scripts/measure_sources.py` runs five titles × four sources and writes JSON + Markdown to `docs/measurements/` (process-group kill on timeout, empty row dir, `--merge`). Measured 2026-11-16 (`docs/measurements/nf-030-2026-11-16.{md,json}`): LibGen 3/5, median 9.7 s; WeLib 0/5, 2.9–25.6 s blocked; Anna's 4/5, median 62.4 s; Ocean 0/5, 148.7–150.7 s Turnstile. LibGen's *Dune* row failed closed in 6.9 s (only collections and non-English editions) and *Pride and Prejudice* fell through to Anna's when its file host stalled. Default order `libgen → welib → annas → ocean` is unchanged and now evidence-backed; Ocean stays explicit-only headless (`NOVELFORGE_OCEAN_HEADFUL=1` is debug-only). Backlog records the licensing supersede decision, extends NF-031 with these decoys and creates NF-032 (lawful public-domain sources); `BooksPage.tsx` source cards carry the measured numbers; `tools.rs` no longer requires Chromium for readiness. Gates: `npm test` 28/28, `cargo test --lib` 150/150, `npx tsc --noEmit` clean, `bash scripts/check.sh` passing. |
| 2026-11-16 | NF-031 DONE: offline book-scoring regression fixture. `scripts/fixtures/libgen/{fourth-wing,dune}.html` are real search pages saved 2026-11-16 (11 and 13 EPUB rows) with `README.md` recording provenance. New `scripts/test_aget.py` (stdlib `unittest`, no network, 21 tests) covers: `parse_libgen` on both pages and an unrelated page; exact English *Fourth Wing* selected at 1.00; the decoy-only *Dune* page returns `None` and writes no file; `choose_candidates` returns only ≥ 0.62 rows, best first, with `limit` honoured and no foreign-language row; title floor, omnibus/collection, punctuation suffix with/without author confirmation, co-author author-score floor, declared non-English rejection and explicit `--lang` selection; Anna's title-only renormalisation (exact passes, fuzzy fails); Ocean `[PDF] [EPUB] … Download` cleanup; and a contract test pinning 0.62 / 0.55 / 0.8. Writing the fixture found a real defect: a French `Fourth wing` scored 0.90 for an English request because language was only a 0.10 weight, so `candidate_score` now returns 0 for a declared non-English row when no `--lang` was requested and for a declared mismatch when one was (an explicit `--lang French` still selects it). `scripts/check.sh` runs the suite after `npm test` (uses `NOVELFORGE_AGET_PYTHON`, then the aget venv, then `python3`); CI installs `click`/`rich` and runs the same suite. Backlog status, summary row, Phase 6 and QA matrix updated. Gates: 21 Python tests, `npm test` 28/28, `cargo test --lib` 150/150, production build, `bash scripts/check.sh` passing. |
|  |  |

---

## Appendix A — Key code references

| Concern | Location |
|---|---|
| Webnovel command construction | `src-tauri/src/process.rs` → `build_args` |
| Webnovel request validation | `src-tauri/src/process.rs` → `validate_request` |
| Progress loop / speed | `src-tauri/src/process.rs` → `progress_loop` |
| Index fetch and source special-casing | `src-tauri/src/metadata.rs` → `fetch_chapter_index` |
| Generic index | `src-tauri/src/metadata.rs` → `fetch_generic_index` |
| NovelFire index | `src-tauri/src/metadata.rs` → `fetch_novelfire_index` |
| NovelPhoenix index | `src-tauri/src/metadata.rs` → `fetch_novelphoenix_index` |
| Paced TOC fetcher | `src-tauri/src/metadata.rs` → `fetch_toc_pages` |
| Search sources | `src-tauri/src/resolver.rs` → `search_novels_from` |
| Source catalogue snapshot | `src-tauri/src/catalogue.rs`, `scripts/build_source_catalogue.py` |
| Source catalogue command | `src-tauri/src/commands.rs` → `list_source_catalogue` |
| Structured export | `src-tauri/src/export.rs` → `export_epub` |
| Borrowed/recovered chapter names | `src-tauri/src/metadata.rs` → `enrich_index_titles`, `adopt_chapter_titles`, `recover_chapter_titles` |
| EPUB heading retitle | `src-tauri/src/export.rs` → `retitle_epub`, `epub_heading` |
| Library scan / loose archive classification | `src-tauri/src/commands.rs` → `list_library`, `library_archives` |
| Legacy archive adoption | `src-tauri/src/export.rs` → `adopt_epub`, `src-tauri/src/commands.rs` → `adopt_archive` |
| Duplicate detection | `src-tauri/src/duplicates.rs` → `find_existing_download` |
| Source URL identity | `src-tauri/src/export.rs` → `normalize_source_url` |
| Update merge | `src-tauri/src/export.rs` → `merge_previous_export` |
| Library novel reader | `src-tauri/src/export.rs` → `library_novel` |
| Library recovery actions | `src-tauri/src/library_actions.rs` |
| Disk accounting / quota / cleanup | `src-tauri/src/storage.rs`, `src-tauri/src/commands.rs` → `list_library`, `cleanup_engine_work`, `delete_partial_downloads` |
| Sandbox containment | `src-tauri/src/paths.rs` → `contained_path`, `is_hidden_path` |
| Target-folder validation / busy guard | `src-tauri/src/process.rs` → `start_download`, `folder_is_busy` |
| Search panel hydration | `src/components/novels/NovelSearchPanel.tsx` |
| Home shelf | `src/pages/HomePage.tsx` |
| Library page | `src/pages/LibraryPage.tsx` |
| Source transport gate / profiles | `src-tauri/src/transport.rs` |
| Manga extractor catalogue | `src-tauri/src/resolver.rs` → `list_manga_sources` |
| Manga command construction | `src-tauri/src/process.rs` → `build_args`, `manga_chapter_filter` |
| Manga packaging | `src-tauri/src/export.rs` → `package_manga_directory` |
| Manga metadata | `src-tauri/src/process.rs` → `gallery_dl_metadata` |
| Source adapter registry / support levels | `src-tauri/src/sources.rs` |
| Author-page resolution | `src-tauri/src/resolver.rs` → `collect_author_pages`, `src-tauri/src/metadata.rs` → `fetch_author_page` |
| lncrawl source patches | `scripts/patches/`, `scripts/apply-lncrawl-patches.sh` |
| Per-host overrides | `src-tauri/src/settings.rs` → `SourceLimit`, `normalize_source_host` |
| Transport report command | `src-tauri/src/commands.rs` → `get_transport_report` |
| Transport UI | `src/pages/GuardrailsPage.tsx`, `src/pages/SettingsPage.tsx` |
| Debug harness | `src-tauri/src/qa.rs` |

## Appendix B — Current output contract

Every successful structured webnovel download should contain:

```text
<download-root>/
  <novel-slug>-<uuid>/
    cover.jpg|png|webp
    metadata.json
    index.json
    chapters.html
    chapters/
      0001 - Chapter Title.xhtml
      0002 - Chapter Title.xhtml
      ...
    <Novel Title>.epub
```

`metadata.json` includes:

- title,
- author/authors,
- synopsis,
- tags,
- status,
- language,
- source URL/source,
- cover filename,
- EPUB filename,
- chapter count,
- downloaded chapter count,
- requested chapter selection,
- index completeness,
- completeness flag,
- full chapter list,
- generated timestamp,
- verification report.

When a source's table of contents leaves rows as bare numbers, the chapter
names are filled by number from a verified donor or recovered from the chapter
page. The bundled EPUB is then retitled to match (`Chapter N - <name>` in
`<title>`, `<h1>`, `nav.xhtml` and `toc.ncx`), so the index, the split XHTML
files and the EPUB never disagree. A book whose source names every row is left
byte-for-byte untouched.

Partial/failed downloads:

- remain on disk,
- have `complete=false`,
- carry `verified.ok=false` and a reason,
- are hidden from the Home shelf,
- remain visible in the Library for inspection.

Manga downloads use the same folder contract with image chapter directories:

```text
<download-root>/
  <series-slug>-<uuid>/
    cover.png
    metadata.json
    index.json
    chapters.html
    chapters/
      0001 - <chapter label>/001.png …
    <Series Title>.cbz
```

The CBZ itself contains one folder per chapter. Alternate uploads of the same
`(language, volume, chapter)` are collapsed to the upload with the most pages.
