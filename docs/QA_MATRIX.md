# NovelForge test matrix

This is the map from "a change landed" to "what proves it". Offline checks are
required for every change; live smoke is opt-in because it depends on third-party
sites and can be slow or rate-limited.

## 1. Offline gates (run on every change)

```bash
scripts/check.sh
```

That runs:

| Layer | Command | Covers |
|---|---|---|
| Rust formatting | `cargo fmt --check` | no unformatted release code |
| Rust lints | `cargo clippy --all-targets -- -D warnings` | warnings are build failures; CI runs the same |
| Rust unit tests | `cargo test --lib` (inside `src-tauri`) | parsers, completeness rules, TTL cache, transport backoff, export, cleanup, library actions, settings, resolver matching and verification waves |
| Rust offline integration | `cargo test --test offline_export` (inside `src-tauri`) | a real fixture EPUB through `audit_epub` + `export_epub`: cleaned chapters, cover, `metadata.json`, `index.json`, `chapters.html`, complete/incomplete flags |
| Book source scoring (Python) | `python3 -m unittest discover -s scripts -t scripts` | `parse_libgen` over two real LibGen pages; title floor, omnibus, prefix/author, language and co-author guards; `choose_candidate`/`choose_candidates` order, limit and fail-closed behaviour; Anna's title-only renormalisation; Ocean heading cleanup; Gutenberg and Standard Ebooks search/book parsing, language mapping, `?source=download` interstitial bypass and HTML-payload rejection |
| lncrawl/NovelFire patch (Python) | `python3 -m unittest discover -s scripts -t scripts` | `test_lncrawl_scraper_patch.py`: patcher idempotence, ambiguous-source rejection, and the saved `readnovel.site` loading stub with its escaped mirror destination |
| Frontend component tests | `npm test` | Home shelf source filter labels, counts, filtering, vanished-source fallback; search partial-result/failure lists, zero hits, empty-query and direct-URL guards, cached-age label; Library incomplete-index warning, migrated entries and exact clicked-folder actions; persisted-task hydration; download size estimate, 300-chapter warning threshold and quota levels; resolver confidence helpers; Books series checklist, per-book selection/fetch, public-domain opt-in selection and source-failure rendering; diagnostics copy via the Clipboard API, its `execCommand` fallback and total failure; About's Ko-fi button calls the backend opener |
| Frontend dead code + unused deps | `npm run knip` | unreferenced files, exports, types and dependencies; wired into `scripts/check.sh` and CI |
| Frontend typecheck + bundle | `npm run build` | TypeScript and Vite production build |
| Linux launcher registration | `bash ./scripts/install-linux-desktop.sh <AppImage>` | copies the bundle to `~/.local/bin`, extracts its icon, writes `~/.local/share/applications/novelforge.desktop` with an absolute `Exec` and `StartupWMClass=novelforge`, refreshes the desktop/icon caches; `--launch` starts the app. `bash -n` clean |
| Release license | `LICENSE`, `package.json`, `Cargo.toml` | the project is GPL-3.0-or-later; both release ZIPs stage `LICENSE`, and the Linux ZIP also carries `scripts/install-linux-desktop.sh` |

`.github/workflows/ci.yml` runs the same steps on Linux.
`.github/workflows/windows-build.yml` runs fmt, clippy, the full Rust test
suite and `tauri build --bundles nsis` on `windows-latest`, and uploads the
installer + `scripts/` + SHA-256 manifest as the `novelforge-windows` ZIP;
`.github/workflows/linux-build.yml` does the same for the AppImage and `.deb`
as `novelforge-linux`, including `LICENSE` and the launcher script. It is the
packaging gate for NF-057. Its
`cargo test` step is also the only place the `#[cfg(windows)]` tests run, so
`spawn::tests::native_path_*` (the `\\?\` strip that fixed the lncrawl SQLite
failure) is proven there rather than on Linux.

### Audit tooling (NF-061)

The dead-code and duplication pass uses one tool per language, all runnable
offline:

| Tree | Tool | Command | Current result |
|---|---|---|---|
| TS/TSX | `knip` (devDependency) | `npm run knip` | clean |
| TS types | `tsc` strict unused checks | `npm run build` (`noUnusedLocals`/`noUnusedParameters`) | clean |
| Rust deps | `cargo-machete` | `cd src-tauri && cargo-machete` | no unused dependencies |
| Rust code | `cargo clippy -D warnings` | `scripts/check.sh` | clean |
| Python | `ruff` F-rules | `ruff check scripts --select F401,F811,F841,F821,F822,F823` | clean |
| Python | `vulture` ≥ 80 % | `vulture scripts --min-confidence 80` | clean (60 % HTMLParser/unittest hits are false positives) |

The same pass removed the unreferenced `HeroArt.tsx`, eight unclaimed CSS
classes, four `cover-*.jpg` files, seven `scripts/fixtures/novelphoenix/*.html`
pages and the duplicate `civil_from_days`, `cache_key_hash`, `is_hidden_path`,
`hostFromUrl` and test `novel()` helpers. Details and counts are in NF-061 of
`SOURCE_PIPELINE_BACKLOG.md`.

Frontend coverage is no longer limited to the Home shelf. NF-025 added the
search failure/partial-result paths, the Library incomplete-index and migrated
UI, exact clicked-folder action assertions and persisted-task hydration. NF-028
added the Books series checklist, per-book selection/Fetch and source-failure
rendering. NF-029 added series shelf/skip behaviour: the checklist skips
complete titles and the backend refuses an already-complete series folder.

### P0 regression map

| Item | Regression test |
|---|---|
| NF-001 throttle → `--rate-limit` | `process::tests::webnovel_throttle_maps_to_lncrawl_rate_limit`, `webnovel_throttle_off_keeps_source_default_pacing` |
| NF-002 truncated TOC must not claim complete | `metadata::tests::completeness_requires_real_evidence`, `ignores_non_chapter_json_ld_item_lists`, `advertised_total_ignores_author_stats` |
| NF-003 search coverage / direct-only failures | `resolver::tests::*`, `catalogue::tests::*`, `sources::tests::search_sources_keep_registry_order` |
| NF-017 transport pacing / 429 / curl fallback | `transport::tests::*` plus the live transport probe below |
| NF-018 resolver confidence, author/alias matching | `resolver::tests::interleaves_candidates_across_sources`, `folds_diacritics_and_full_width_forms`, `metadata::tests::parses_aliases_from_json_ld_and_labelled_rows`, `src/lib/resolver.test.ts` |
| NF-026 author-page bibliography | `sources::tests::author_pages_are_declared_with_a_slug_style`, `author_slug_filters_path_characters`, `metadata::tests::author_page_reads_only_the_author_shelf`, `author_page_prefers_json_ld_item_list`, `resolver::tests::author_page_urls_use_the_source_spelling`, `author_page_hits_keep_shelf_order_and_prefer_ready_sources`, `author_page_resolution_lists_the_whole_shelf_once` |

Only P0 issues are listed here. P1/P2 coverage is recorded next to each NF entry
in `SOURCE_PIPELINE_BACKLOG.md`.

### P1/P2 regression map

| Item | Regression test |
|---|---|
| NF-025 frontend interaction coverage | `src/components/novels/NovelSearchPanel.test.tsx` (5 tests), `src/pages/LibraryPage.test.tsx` (9 tests), `src/hooks/useDownloadManager.test.tsx` (1 test) |
| NF-027 resolver cache and verification waves | `cache::tests::round_trips_and_expires`, `a_stored_key_never_answers_for_another_key`, `resolver::tests::exact_author_in_the_first_wave_stops_after_three_fetches`, `a_first_wave_without_an_exact_author_keeps_verifying`, `an_empty_author_page_is_trusted_only_when_path_and_label_agree`, `search_cache_keys_normalise_spelling`, `cache_stamp_keeps_the_oldest_age_and_marks_cached`, `NovelSearchPanel.test.tsx` cached-age label test |
| NF-028 book catalogue, series resolution and verified download | `books::tests::parses_a_hardcover_book`, `parses_typesense_hits`, `parses_series_positions_including_fractional`, `reads_positions_from_edition_series_strings`, `matches_series_names_without_accepting_prefix_noise`, `future_release_dates_are_upcoming`, `parses_openlibrary_series_rows`, token round-trip/rejection; `process::tests::book_args_carry_verified_metadata_and_libgen_source`, `book_source_rejects_unknown_sources`, `book_validation_rejects_malformed_isbn`; `BooksPage.test.tsx` (2 tests: “fetches a single catalogue record with its metadata”, “reports failed catalogue sources instead of inventing rows”; series/PD tests listed under NF-029/NF-032). Scoring/fail-closed behaviour is proven live below; offline fixture is NF-031 |
| NF-029 series layout, skip-complete and not-found | `process::tests::series_layout_uses_author_series_and_padded_position` (`1` → `01`, `3.5` → `03.5`), `series_layout_falls_back_without_a_position_and_stays_contained` (no position → bare title, `../` and `/` contained under root); `BooksPage.test.tsx` (2 series tests, incl. “skips series books that are already complete in the library”). Live layout/skip/not-found probes below |
| NF-030 source budgets, fast-fail and measurement | Live `scripts/measure_sources.py` (five titles × four sources, JSON+Markdown); offline scoring edge cases are covered by NF-031 |
| NF-031 offline book-scoring fixture | `scripts/test_aget.py` (31 tests): `LibGenFixtureParsing`, `SelectionOnRealPages`, `ScoringGuards`, `TitleOnlySources`, `SourceHeadingCleanup`, `ThresholdContract`, `PublicDomainSources`; fixtures `scripts/fixtures/libgen/{fourth-wing,dune}.html`, `scripts/fixtures/gutenberg/*`, `scripts/fixtures/standardebooks/*`; run by `scripts/check.sh` |
| NF-032 opt-in public-domain sources | `scripts/test_aget.py::PublicDomainSources` (auto-order exclusion, search/book parsing, language mapping, non-English rejection, interstitial bypass, plain-EPUB preference, constructed-URL fallback, HTML-payload rejection); `process::tests::book_source_accepts_public_domain_sources`; `BooksPage.test.tsx` “uses an explicitly selected public-domain source”. Live: Gutenberg *Pride and Prejudice* 24,864,480 B / *The Time Machine* 307,155 B verified; Standard Ebooks *Pride and Prejudice* 831,932 B and series *Barchester Towers* shelved under the NF-029 layout; both fail closed on *Dune* |
| NF-033 NovelFire escaped mirror redirect | `scripts/test_lncrawl_scraper_patch.py` (4 tests): idempotent source patch, ambiguous-source rejection, saved `readnovel.site` stub, escaped-destination decode; `process::tests::error_tail_*` keeps the engine's failure line. Live: app QA ch 7–8 2/2 in 293 s (74,870-byte EPUB verified); direct 20/20 in 342 s |
| NF-034 MangaDex licensed chapters | `process::tests::mangadex_licensing_names_one_external_chapter`, `mangadex_licensing_counts_far_past_the_line_buffer`, `unrelated_errors_do_not_claim_a_licensing_gap` (the counter is not limited by the 24-line error ring). Live: a licensed title reports the chapter number and publisher host instead of a bare exit code; the publisher page is never requested |
| NF-035 crawl watchdog and single retry | `process::stall_tests` (9 tests): 900 s → 300 s bound by phase; paused time excluded; a **real `sleep 30` child is killed** and reported `Stalled`; a lone cover is not output but a chapter/EPUB/work-dir chapter is; transient vs `Unsupported URL`/`No crawler` tails; `retry_action` gives Resume for a stall (with or without output), Restart only for a zero-output transient, never for attempt 2/Manga/Completed/unknown; `--all` → `--resume`. Live recipe below (`NF-035` section): good 2/2 run in 47 s, bad URL one launch, transient stub exactly 2 launches, fatal stub exactly 1, stall stub resumes once |
| NF-036 single-hop source preference | `sources::tests::measured_cheaper_source_wins_the_tie_break` (NovelPhoenix ranks before NovelFire; every known host has a finite rank; unknown stays `u8::MAX`), `sources::tests::search_sources_keep_registry_order` (UI order unchanged). Live: `NOVELFORGE_QA_RESOLVE` picks novelphoenix at confidence 1.00; index 2,334/2,334 in 42.0 s; lncrawl 59-page TOC + 20 chapters 20/20 in 56 s |
| NF-037 full-length single-hop run | Live only (no new code): *Reverend Insanity* 2,334/2,334, crawl 1,113.8 s (~2.1 ch/s), 0×429/0×503/0×1015, 0 retries, 33,718,857-byte EPUB `verified=true` with cover, `complete:true`, total 19m 12.6s |
| NF-038 quadratic title pass in export | Existing `export::tests` title/merge coverage; live full-length run exports **16.0 s** (was >37 min stalled at `Exporting 100 %`), writes `metadata.json`/`index.json`/`chapters.html`/`cover.jpg` and reaches `Completed`; 20-chapter smoke export 0.2 s |
| NF-039 live chapter structuring | `structurer::tests::structures_a_real_lncrawl_frame_during_the_crawl`, `a_partial_frame_is_retried_until_the_writer_finishes`, `ignores_translations_and_non_zstd_files`, `sweep_only_removes_provisional_names_the_export_replaced`; `chapter_clean::tests::void_elements_are_self_closing_so_the_xhtml_parses`. Live: files appear during `DownloadingChapters` (5→40), export 0.4 s, 40/40, no provisional leftovers, `xmllint` clean |
| NF-040 pacing ratchet recovery | `scripts/test_lncrawl_scraper_patch.py` (7 tests): previous 4 + idempotent `Pacer.learn` rewrite, missing-body rejection, exactly one store assignment behind the guard. `patch_lncrawl_scraper.py --verify` runs a real `Pacer` simulation: `learn(9.8)` → ten `eased()` steps narrows to 8.82 s, a stale `learn(9.8)` keeps 8.82 s, and a fresh origin still adopts 3.0 s. Simulated 500 requests: stock 5.930 s vs patched 0.500 s. Live profile before the fix: `novelfire.net 9.801 s / 401 successes` vs `novelphoenix.net 0.333 s` |
| NF-041 measured pace and faster source | `export::tests::measured_crawl_pace_is_persisted_and_exposed_to_the_library` (pace round-trips through `metadata.json`, `library_novel` exposes host/rank/pace, zero chapters records nothing), `process::tests::expected_pace_rounds_up_and_needs_a_measurement`, `process::tests::crawled_chapter_count_uses_the_run_selection_not_the_merged_export` (update `20,21` of 53 counts 2, not 53); `LibraryPage.test.tsx` “shows the measured pace on the card”, “offers a verified cheaper host as a new download and never switches”, “keeps the current source when the dialog is dismissed”. Live recipe below: 94.4 ch/min full run, two-chapter update `expected 94.38 · 2 s`, NovelFire rank 5 → NovelPhoenix rank 0, NovelPhoenix → already fastest |
| NF-042 stdout fatal reasons | `process::tests::stdout_traceback_names_the_actionable_reason`, `stdout_progress_and_info_lines_are_not_errors`, `stdout_exhausted_line_is_still_a_transient_failure`, `zero_output_message_keeps_a_precise_stdout_reason`. Live recipe below: real bad URL and a stdout-exit-0 stub each launch **once** and report `Source error · … No crawler found for the domain`; transient stderr still retries twice |
| NF-043 partially-numbered TOC names | `metadata::tests::bare_chapter_title_matches_numbering_only_titles`, `donor_chapter_name_strips_repeated_numbering`, `display_chapter_number_prefers_the_visible_number`, `adopt_chapter_titles_matches_by_visible_number_and_keeps_existing_names` (names filled, source names and URLs untouched), `adopt_chapter_titles_rejects_low_coverage_and_changes_nothing`, `adopt_chapter_titles_rejects_a_shifted_donor` (one-number offset is refused), `title_lookup_runs_only_when_the_source_leaves_rows_unnamed` (mixed index triggers, fully named book does not, one stray row is page-recovered instead), `chapter_header_names_are_recognised_from_content`, `chapter_name_is_read_from_the_content_container` (labelled header, unlabelled name line, prose rejected); `export::tests::borrowed_chapter_names_are_written_into_the_bundled_epub` (`<title>`/`<h1>`/nav/NCX rewritten, source-named row untouched, archive still audits), `bundled_epub_is_untouched_when_every_row_was_named`. Live recipe below: RI 379/379, ORV 65/65, LOTM unchanged |
| NF-044 visible number vs lncrawl serial | `process::tests::webnovel_selection_is_translated_from_visible_numbers_to_serials` (ORV visible `2-3` → serials `3-4`; 1-based source unchanged), `webnovel_selection_fails_closed_without_a_matching_index_row` (unlisted `111` and a missing index both refuse), `webnovel_selection_refuses_an_incomplete_or_ambiguous_index`, `export::tests::audit_rejects_a_selection_that_fetched_the_neighbouring_chapter` (headings `1,2` for selection `2-3` fail with `position 1 is Chapter 1, expected Chapter 2`; unnamed headings and volume restarts still pass), `structurer::tests::sweep_maps_a_serial_to_the_visible_chapter_the_export_wrote`, `library_actions::tests::full_spec_covers_every_listed_chapter` (ORV `0-3,5`), 3 `parseChapterSpec` frontend tests (chapter `0` accepted). Live recipe below: ORV `2-3` 2/2 with exactly `0002`/`0003`, ORV `275-277` 3/3, ORV `111` fails before any folder, RI `1-2` 2/2 unchanged |
| NF-045 atomic durable writes | `fsutil::tests::atomic_write_replaces_content_and_leaves_no_temp`, `a_failed_replace_removes_its_temporary`; `settings`/`export`/`metadata`/`structurer` now call `fsutil`. Windows replace is `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` |
| NF-046 quit guard | Backend: `CloseRequested`/`ExitRequested` prevent exit while `active_job_count() > 0`, emit `app://quit-requested`; `confirm_quit` sets `allow_exit`. Frontend: `useDownloadManager` listens and App renders the modal (`Stop and quit` / `Keep downloading`). Manual UI check required (no headless Tauri test) |
| NF-047 job journal / interrupted resume | `process::journal_tests::a_running_job_is_restored_as_interrupted` (also asserts the boot rewrite persists `interrupted`, not `running`), `completed_jobs_keep_their_status_across_a_restart`, `the_journal_is_schema_stamped`; frontend `useDownloadManager.test.tsx` “offers a resume action for a job interrupted by a restart”. Resume path is `resume_interrupted` → verified `retry_download`. Live: one-row `running` journal restored and rewritten at boot; Downloads showed the interruption message and Resume control |
| NF-048 persistent log + diagnostics | `process::journal_tests::civil_dates_match_known_epochs` proves the UTC formatter; `emit_log` appends to `novelforge.log` with 5 MiB rotation; `get_diagnostics` returns a redacted report. Manual: About → **Copy diagnostics** includes tool paths and the last 200 lines, home paths shortened to `~` |
| NF-049 error boundary | `src/components/ErrorBoundary.tsx`; manual: force a render throw, confirm Reload and Copy error are usable |
| NF-050 single instance | `tauri-plugin-single-instance`; manual: second launch focuses the first window and does not create a second config writer |
| NF-051 Windows process control | `terminate_process`/`force_kill_process` (`taskkill /PID /T [/F]` on Windows); pause/resume are now real on Windows via NF-065. Windows runtime proof is NF-057 |
| NF-052 pinned/modular toolchain | `scripts/install-tools.sh` accepts `webnovel`, `manga`, `books`, `all`; pins lncrawl at commit `59b0382` (`dev`, reports 4.14.0 — tag `v4.14.0` predates the refactor patch 6 needs), `gallery-dl 1.32.20`, `click 8.5.1`, `nodriver 0.50.99`, `playwright 1.65.0`, `rich 15.0.0`; `bash -n` clean; native MSYS/Cygwin exits explicitly. `scripts/install-tools.ps1` is the Windows counterpart with the same pins and patch order, `%LOCALAPPDATA%\novelforge\bin` shims and `-SkipBrowser`; `tools::tests` cover the shim lookup, missing configured paths, tool-id mapping and the Playwright Chromium layout. Tool checks name the searched `%LOCALAPPDATA%\novelforge\bin`, Settings offers a per-field **Check** (`probe_tool`) and **Re-check** probes unsaved form values; both return a visible notice. Both installers normalise patch files to LF (`.gitattributes` marks `*.patch -text`), verified with a CRLF copy of `scripts/patches/`: 8/8 applied, then 8/8 already applied |
| NF-053 fmt/clippy gate | `scripts/check.sh` and `.github/workflows/ci.yml` run `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`; both pass on the batch |
| NF-054 ZIP asset path | `export::safe_entry_path` rejects absolute paths, drive prefixes and `..`; used for every copied asset entry |
| NF-055 schema version | `journal_tests::the_journal_is_schema_stamped`; `metadata.json`, `index.json`, `settings.json` and `jobs.json` carry `schemaVersion` on write |
| NF-056 corrupt bundled catalogue | `catalogue::entries` logs the parse error and returns an empty list instead of `panic!`; the app still opens and the source picker is empty |
| NF-060 personal-data / fixture guard | `scripts/check-personal-data.sh` runs first in `check.sh`: product inputs and docs/CI/test sources must not contain real `/home/<user>/` or `C:\Users\<user>\` paths (placeholders such as `test`, `example` and CI's `runner` are allowed), `tauri.conf.json` must not declare `bundle.resources`, release code may only `include_bytes!`/`include_str!` from `resources/` before the first `#[cfg(test)]`, known fixture markers must be absent, and an existing release binary is grepped for fixture prose. Probe: adding `/home/<user>/probe/` to `src/__guard_probe.ts` failed the gate until removed |
| NF-062 no console windows on Windows | `src-tauri/src/spawn.rs` wraps every child: `hide_console` (`tokio`) and `hide_console_std` (`std`) set `CREATE_NO_WINDOW`. Used by `tools::probe`, `CrawlSpawner::spawn_with`, `run_lncrawl_retry`, both `resolver` searches, both `transport` curl calls and the two `taskkill` helpers. Manual on Windows: no window at start-up, during a fetch, or during a download |
| NF-063 lncrawl SQLite path | `process::stall_tests::lncrawl_env_pins_the_database_to_the_work_dir` asserts both env vars and the `sqlite:///…/sqlite.db` shape; `spawn::tests::native_path_*` (Windows CI) assert `\\?\C:\…` → `C:\…`, `\\?\UNC\server\share` → `\\server\share`, and that volume GUIDs/plain paths are untouched. Manual on Windows: the job log prints the database URL and 2 chapters download |
| NF-064 diagnostics copy fallback | `src/lib/clipboard.test.ts` (3 tests): Clipboard API path, `execCommand` fallback after a rejected `writeText`, total failure with no leftover textarea. Manual: About shows the report in a textarea when the clipboard is blocked |
| NF-065 Windows pause/resume | `spawn::tests::pause_stops_the_whole_tree_and_resume_releases_it` (Windows CI: `cmd /C ping -n 3` is the launcher+worker pair; the tree is still alive 2.5 s after the pause and finishes after resume), `pause_refuses_a_process_that_is_already_gone` (pid 0 is refused on both platforms). `process::pause_process`/`resume_process` call the tree calls under `#[cfg(windows)]`; Unix keeps `SIGSTOP`/`SIGCONT` on the group |
| NF-066 comic archive is the deliverable | `export::tests::library_prefers_the_archive_named_after_the_title` (title-named CBZ beats a larger stray ZIP; a per-chapter archive is the fallback; no archive is not an error). The series-wide shape is superseded by NF-068 |
| NF-067 TOC walk visibility + one walk per book | `metadata::tests::one_walk_lock_per_index_key` (same key → same lock, different key → different lock). Manual: picker shows `TOC page N / M`; pressing Start while the index loads does not restart or duplicate the walk |
| NF-068 one CBZ per manga chapter | `export::tests::packages_gallery_dl_tree_into_one_cbz_per_chapter` (two `chapters/NNNN - label.cbz` files, no expanded folders, no series archive, 4 pages over 2 archives), `manga_packaging_prefers_one_upload_per_chapter` (duplicate scan-group upload → one archive), `rebuilds_chapter_archives_from_the_page_tree` (legacy expanded folders → per-chapter archives, folder removed, natural order), `splits_a_legacy_series_archive_into_chapter_archives` (0.1.2 series CBZ → 2 chapter CBZs, 5 pages, replaced copy deleted), `commands::tests::repack_builds_per_chapter_archives_and_refuses_a_novel`, `library_counts_only_per_chapter_archives` (legacy series archive → count 0 and still openable; two chapter archives → count 2). Manual: Library shows **Open CBZs**; log shows `Comic archives · N of M verified` |
| NF-069 book failures name the sources | `process::tests::book_failure_names_the_aget_sources` (per-source lines composed into the job message, search chatter dropped, generic fallback still actionable), `aget_source_lines_ignore_search_chatter` (`skip_note`/`No download completed` accepted, `searching…` and row-count lines rejected). `tools::probe_aget` runs `aget --help`; manual: a failed book row shows `aget: …`, the same text is in `novelforge.log`. **Book downloads are an experiment, not a release gate** |
| NF-070 manga progress counts pages | `process::tests::manga_progress_counts_pages_inside_the_active_chapter` (1 finished chapter + 1/10 of the active one = 27.5% of 4, not 50%), `manga_progress_falls_back_to_finished_chapter_average` (no `count` → average of finished chapters), `manga_progress_without_a_page_baseline_does_not_invent_one` (first chapter keeps 0 instead of assuming a page count), `gallery_info_page_count_reads_the_extractor_count` (number, string and missing `count`), `work_stats_counts_manga_chapters_and_pages` (active folder, expected pages, finished pages). Manual: a 4-chapter manga selection advances page by page through chapter 2 instead of jumping to 50% on its first image |
| NF-071 clear finished terminals | `useDownloadManager.test.tsx::clears finished jobs while keeping the active row` (`clearFinished` removes the completed row, keeps the running row, keeps its output). Manual: Downloads → Terminal → **Clear finished** removes old rows and their lines; active output continues |

## 2. Live smoke (opt-in)

Run only when a source adapter, parser, resolver or transport profile changed.
The harness is the debug build in `qa.rs`; it drives the real pipeline without
the webview.

```bash
NOVELFORGE_ALLOW_LIVE=1 scripts/source-smoke.sh
# or one target
NOVELFORGE_ALLOW_LIVE=1 scripts/source-smoke.sh divinedaolibrary
```

Each run downloads chapters 1–2 into a timestamped directory under
`/tmp/opencode/` and writes one log per source. A passing run must show:

- a resolved title,
- a `Chapter index`/`Chapter audit` line with the expected count,
- `2/2 chapters present and non-empty`,
- `Structured export · metadata.json + index.json + chapters.html`,
- `Success — 1 verified output file(s)`,
- a final `transport report` with 0 blocked hosts and any 429/retry counts.

### Source matrix

| Target | URL | Why it is in the matrix |
|---|---|---|
| `novelphoenix` | `https://novelphoenix.net/mother-of-learning.html` | first-class parser, paginated TOC |
| `novelfull` | `https://novelfull.net/reverend-insanity.html` | AJAX catalogue, curl transport, 59 TOC pages |
| `freewebnovel` | `https://freewebnovel.com/novel/reverend-insanity` | generic parser on a first-class host |
| `divinedaolibrary` | `https://www.divinedaolibrary.com/story/alchemy-master/` | WordPress/Fictioneer markup, template-free TOC |
| `novelping` | `https://novelping.com/book/prison-of-eternal-sky` | direct-URL-only source, AJAX archive |
| `novelfire` | `https://novelfire.net/book/reverend-insanity` | transport profile with 429 history (index-only is acceptable) |

The matrix covers at least three first-class sources, a generic parser, a
paginated TOC, an AJAX catalogue, a WordPress/Fictioneer source and a
direct-URL-only host.

### Other live probes

```bash
# Chapter index only, with a forced fresh walk
(cd src-tauri && NOVELFORGE_QA_INDEX_URL="https://novelfull.net/reverend-insanity.html" \
  cargo run --quiet)

# Search behaviour and per-source failures
(cd src-tauri && NOVELFORGE_QA_SEARCH="Reverend Insanity" \
  NOVELFORGE_QA_SEARCH_SOURCES="https://novelfire.net/,https://novelphoenix.net/" \
  cargo run --quiet)

# Resolver: title, author mode, author prefix, alias and no-match paths
(cd src-tauri && NOVELFORGE_QA_RESOLVE="reverend insanity" cargo run --quiet)
(cd src-tauri && NOVELFORGE_QA_RESOLVE="Gu Zhen Ren" \
  NOVELFORGE_QA_RESOLVE_MODE=author cargo run --quiet)
(cd src-tauri && NOVELFORGE_QA_RESOLVE="author: Gu Zhen Ren" cargo run --quiet)
(cd src-tauri && NOVELFORGE_QA_RESOLVE="master of gu" cargo run --quiet)
(cd src-tauri && NOVELFORGE_QA_RESOLVE="qwzxv plorbnak zzz" cargo run --quiet)

# Optional: limit which searchable sources the resolver queries
NOVELFORGE_QA_RESOLVE_SOURCES="https://novelphoenix.net/" ... cargo run --quiet

# Optional: print each candidate's parsed title/author/score during verification
NOVELFORGE_QA_RESOLVE_DEBUG=1 NOVELFORGE_QA_RESOLVE="Gu Zhen Ren" \
  NOVELFORGE_QA_RESOLVE_MODE=author cargo run --quiet

# Author page proof: NovelFire lists four Gu Zhen Ren novels, NovelPhoenix two.
# Expect four titles (and 1.5–2.5 s) for the default selection, with no
# crawler search process in author mode.
NOVELFORGE_QA_RESOLVE="Gu Zhen Ren" NOVELFORGE_QA_RESOLVE_MODE=author \
  cargo run --quiet
NOVELFORGE_QA_RESOLVE="Gu Zhen Ren" NOVELFORGE_QA_RESOLVE_MODE=author \
  NOVELFORGE_QA_RESOLVE_SOURCES="novelfire.net" cargo run --quiet
NOVELFORGE_QA_RESOLVE="Gu Zhen Ren" NOVELFORGE_QA_RESOLVE_MODE=author \
  NOVELFORGE_QA_RESOLVE_SOURCES="novelphoenix.net" cargo run --quiet

# Low-confidence guard: must fail before creating a download folder
(cd src-tauri && NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/resolve-guard \
  NOVELFORGE_QA_QUERY="reverend insanity 2" cargo run --quiet)

# Transport/backoff/curl probe for one URL
(cd src-tauri && NOVELFORGE_QA_TRANSPORT_URL="https://novelfull.net/reverend-insanity.html" \
  cargo run --quiet)

# Pause/resume path (opt-in, needs a live job)
(cd src-tauri && NOVELFORGE_QA_QUERY="https://novelphoenix.net/mother-of-learning.html" \
  NOVELFORGE_QA_CHAPTERS="1-8" \
  NOVELFORGE_QA_PAUSE_AT=40 \
  NOVELFORGE_QA_PAUSE_SECONDS=5 \
  cargo run --quiet)

# Storage totals, then each explicit cleanup action
(cd src-tauri && NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/storage-check \
  NOVELFORGE_QA_STORAGE=report cargo run --quiet)
(cd src-tauri && NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/storage-check \
  NOVELFORGE_QA_STORAGE=work cargo run --quiet)
(cd src-tauri && NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/storage-check \
  NOVELFORGE_QA_STORAGE=partials cargo run --quiet)

# Quota gate must fail before the resolver touches the network
(cd src-tauri && NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/storage-check \
  NOVELFORGE_QA_QUOTA_BYTES=1 \
  NOVELFORGE_QA_QUERY="https://novelphoenix.net/mother-of-learning.html" \
  NOVELFORGE_QA_CHAPTERS=1-2 cargo run --quiet)

# Keep the crawler work tree after a success to inspect the toggle
NOVELFORGE_QA_KEEP_WORK=1 NOVELFORGE_QA_QUERY=... cargo run --quiet
```

Cancellation is verified through the same live job: start it, press Cancel in
the app, then confirm the job ends `Cancelled`, the work tree remains, and the
destination has no half-written structured export. This remains a live/manual
check because it exercises the OS process supervisor rather than pure Rust
logic.

### Books catalogue, series and download (NF-028)

```bash
# Catalogue + series resolution (Open Library, no token needed)
(cd src-tauri && NOVELFORGE_QA_BOOKS="The Empyrean" cargo run --quiet)
(cd src-tauri && NOVELFORGE_QA_BOOKS="The Lord of the Rings" cargo run --quiet)
(cd src-tauri && NOVELFORGE_QA_BOOKS="Dune" NOVELFORGE_QA_BOOKS_MODE=books cargo run --quiet)

# Force a cold catalogue run
rm -rf ~/.local/share/novelforge/book-cache

# Verified free-source download through the app path (LibGen, no browser)
(cd src-tauri && NOVELFORGE_QA_QUERY="fourth wing" NOVELFORGE_QA_ENGINE=books \
  NOVELFORGE_QA_BOOK_TITLE="Fourth Wing" NOVELFORGE_QA_BOOK_AUTHOR="Rebecca Yarros" \
  NOVELFORGE_QA_BOOK_ISBN="9781649374042" NOVELFORGE_QA_BOOK_SOURCE=libgen \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf-books-download cargo run --quiet)

# Fail-closed: wrong expectation must write no file and report near misses
(cd src-tauri && NOVELFORGE_QA_QUERY="fourth wing" NOVELFORGE_QA_ENGINE=books \
  NOVELFORGE_QA_BOOK_TITLE="The Hobbit" NOVELFORGE_QA_BOOK_AUTHOR="J.R.R. Tolkien" \
  NOVELFORGE_QA_BOOK_SOURCE=libgen NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf-books-miss \
  cargo run --quiet)
```

### Series layout, skip-complete and not-found (NF-029)

```bash
# First run: shelf as <Author>/<Series>/<NN> - <Title>/
(cd src-tauri && NOVELFORGE_QA_QUERY="fourth wing" NOVELFORGE_QA_ENGINE=books \
  NOVELFORGE_QA_BOOK_TITLE="Fourth Wing" NOVELFORGE_QA_BOOK_AUTHOR="Rebecca Yarros" \
  NOVELFORGE_QA_BOOK_ISBN="9781649374042" NOVELFORGE_QA_BOOK_SERIES="The Empyrean" \
  NOVELFORGE_QA_BOOK_POSITION=1 NOVELFORGE_QA_BOOK_SOURCE=libgen \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf-029-download cargo run --quiet)
find /tmp/opencode/nf-029-download -maxdepth 4 -type f -not -path '*/chapters/*' | sort

# Second run: must refuse the existing complete folder (no duplicate)
(cd src-tauri && NOVELFORGE_QA_QUERY="fourth wing" NOVELFORGE_QA_ENGINE=books \
  NOVELFORGE_QA_BOOK_TITLE="Fourth Wing" NOVELFORGE_QA_BOOK_AUTHOR="Rebecca Yarros" \
  NOVELFORGE_QA_BOOK_ISBN="9781649374042" NOVELFORGE_QA_BOOK_SERIES="The Empyrean" \
  NOVELFORGE_QA_BOOK_POSITION=1 NOVELFORGE_QA_BOOK_SOURCE=libgen \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf-029-download cargo run --quiet)

# Library scan must see the nested novel as complete
(cd src-tauri && NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf-029-download \
  NOVELFORGE_QA_LIBRARY=1 cargo run --quiet)
```

Expected evidence:

| Case | Pass condition |
| --- | --- |
| Series shelf | `…/Rebecca Yarros/The Empyrean/01 - Fourth Wing/01 - Fourth Wing.epub` (≈ 4.83 MB) plus `metadata.json`, `index.json`, `chapters.html`, cover and `chapters/`; log shows `Shelved as 01 - Fourth Wing.epub` |
| Unpositioned book | same author/series folders but no `NN - ` prefix |
| Repeat run | `start failed: “Fourth Wing” is already complete in the library at …/01 - Fourth Wing`; no new folder or EPUB |
| Library scan | one nested novel, `complete: true`, `diskBytes` ≈ 17.7 MB, `epubPath` = `01 - Fourth Wing.epub` |
| Not found | when every mirror times out, the job ends `Book not found: no catalogue-verified EPUB was produced (network failure, mirror block, or no row cleared the match threshold)…` and writes no EPUB |

### Opt-in public-domain sources (NF-032)

```bash
# Project Gutenberg: verifies dcterms:language on the book page, then .epub3.images
(cd src-tauri && NOVELFORGE_QA_QUERY="The Time Machine" NOVELFORGE_QA_ENGINE=books \
  NOVELFORGE_QA_BOOK_TITLE="The Time Machine" NOVELFORGE_QA_BOOK_AUTHOR="H. G. Wells" \
  NOVELFORGE_QA_BOOK_SOURCE=gutenberg \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf-032-gut cargo run --quiet)

# Standard Ebooks: plain EPUB with ?source=download (the bare URL is an interstitial)
(cd src-tauri && NOVELFORGE_QA_QUERY="Pride and Prejudice" NOVELFORGE_QA_ENGINE=books \
  NOVELFORGE_QA_BOOK_TITLE="Pride and Prejudice" NOVELFORGE_QA_BOOK_AUTHOR="Jane Austen" \
  NOVELFORGE_QA_BOOK_SOURCE=standardebooks \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf-032-se cargo run --quiet)

# Fail-closed: a modern title is absent from both catalogues
(cd src-tauri && NOVELFORGE_QA_QUERY="Dune" NOVELFORGE_QA_ENGINE=books \
  NOVELFORGE_QA_BOOK_TITLE="Dune" NOVELFORGE_QA_BOOK_AUTHOR="Frank Herbert" \
  NOVELFORGE_QA_BOOK_SOURCE=gutenberg \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf-032-miss cargo run --quiet)
```

Expected evidence:

| Case | Pass condition |
| --- | --- |
| Gutenberg | `output … (… bytes, kind=epub, verified=true)`; `mimetype` is the first ZIP entry and stored; log shows a `best match` score ≥ 0.62 |
| Standard Ebooks | same verified output; the fetched URL ends in `?source=download` (an HTML interstitial must never be saved) |
| Missing title | `no verified EPUB`/`no result`, no file in the download directory |
| Auto order | selecting `gutenberg`/`standardebooks` never changes what `auto` tries; `resolve_sources("auto") == SOURCES` |

Expected evidence:

| Case | Pass condition |
| --- | --- |
| `The Empyrean` | series *The Empyrean* / Rebecca Yarros; Fourth Wing #1, Iron Flame #2, Onyx Storm #3; ~8 s cold, cached on repeat |
| `The Lord of the Rings` | Fellowship #1, Two Towers #2, Return of the King #3, The Hobbit unpositioned; companion/omnibus rows dropped; ~15 s cold |
| Catalogue rows | 20 candidates with title/authors/year/ISBN where available; every row has a source and stable id |
| Download (Fourth Wing) | `Fourth Wing.epub` ≈ 4.83 MB plus `metadata.json`, `index.json`, `chapters.html`, `cover.jpg`, chapter XHTML, `Images/`, `Styles/`; log shows a scored best match, not the first row |
| Wrong expectation | no EPUB written; log/result lists near misses and the threshold that failed |
| Default path | no Chromium/`nodriver` process starts for a LibGen run |

Hardcover is optional and needs a user token stored locally at
`<app config>/hardcover-token`; without it the same probes must pass on Open
Library alone. Ocean/Anna's/WeLib timing stays under NF-030.

### NovelFire mirror redirect (NF-033)

```bash
# NovelFire answers chapter requests with a 302 to readnovel.site, whose
# "Loading..." page redirects (with JSON-escaped slashes) to novelphoenix.com.
# Confirm the hop is still live, then download chapters that used to 404.
curl -sS -o /dev/null -D - -A "Mozilla/5.0 (X11; Linux x86_64) Chrome/131" \
  "https://novelfire.net/book/reverend-insanity/chapter-7" | head -5

rm -rf /tmp/opencode/nf-033 && mkdir -p /tmp/opencode/nf-033
LNCRAWL_DATA_PATH=/tmp/opencode/nf-033/data \
  ~/.local/bin/lncrawl -ll crawl https://novelfire.net/book/reverend-insanity \
  -f epub --noin --rate-limit 2 --chapters 1-20 2>&1 | tee /tmp/opencode/nf-033/run.log
```

Expected evidence:

| Case | Pass condition |
| --- | --- |
| Escaped stub | `scraper.diagnosis.js_redirect` returns `https://novelphoenix.com/...`, never a URL containing `\/` |
| Install gate | `patch_lncrawl_scraper.py` is idempotent and `--verify` passes for both patches (escaped redirect fixture; `Pacer` ratchet simulation) |
| 7–8 | both chapters present and non-empty; no `HTTPError('404 … readnovel.site…')` line |
| 1–20 | 20/20 chapters in the EPUB; the log has no `readnovel.site` or `novelphoenix.com` 404 |
| Startup | no `Sync online sources` line; the crawl starts within ~2 s of the config/database banners |

### MangaDex licensing gap (NF-034)

```bash
# Solo Leveling still advertises 200 chapters and `en`, but every chapter in
# the English aggregate is licensed to an official publisher. gallery-dl says so
# once per language and exits non-zero.
rm -rf /tmp/opencode/nf034 && mkdir -p /tmp/opencode/nf034
timeout 180 gallery-dl --range 1 --directory /tmp/opencode/nf034 \
  "https://mangadex.org/title/32d76d19-8a05-4db0-9fc2-e0b0648fe9d0/solo-leveling" \
  > /tmp/opencode/nf034/run.log 2>&1; echo "exit=$?"
grep "is not available on MangaDex" /tmp/opencode/nf034/run.log

# Same URL through the app pipeline: the job message must explain licensing.
rm -rf /tmp/opencode/nf034-qa
NOVELFORGE_QA_QUERY="https://mangadex.org/title/32d76d19-8a05-4db0-9fc2-e0b0648fe9d0/solo-leveling" \
  NOVELFORGE_QA_ENGINE=manga NOVELFORGE_QA_CHAPTERS=1 \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf034-qa \
  timeout 300 src-tauri/target/debug/novelforge
```

Expected evidence:

| Case | Pass condition |
| --- | --- |
| Exit code | `exit=4` — gallery-dl treats a licensed chapter as a skip, not a crash |
| Engine line | `Chapter 1 is not available on MangaDex … at https://…` names the official publisher |
| App message | `MangaDex licensing: chapter 1 is external-only and must be read at …`; no bare `Process exited with code 4` |
| Count | one chapter reported even when four language editions each print a line |
| Politeness | the publisher host is never requested — only named |
| Guardrails | the licensing rule is listed under Integrity policy |

Fixtures are not reusable here because the directory depends on live licensing,
so the offline gate is `process::tests::mangadex_licensing_*`.

### Single-hop source preference (NF-036)

```bash
# NovelPhoenix answers a chapter in one request; NovelFire needs a
# 302 -> readnovel stub -> mirror chain. Both carry Reverend Insanity.
NOVELFORGE_QA_RESOLVE="Reverend Insanity" NOVELFORGE_QA_RESOLVE_MODE=webnovel \
  timeout 300 src-tauri/target/debug/novelforge

# Fresh index (59 TOC pages) for the one-hop host.
NOVELFORGE_QA_INDEX_URL="https://novelphoenix.net/reverend-insanity.html" \
  timeout 600 src-tauri/target/debug/novelforge

# End to end: resolve by title, download 20 chapters, export + verify.
rm -rf /tmp/opencode/nf036-qa
NOVELFORGE_QA_QUERY="Reverend Insanity" NOVELFORGE_QA_ENGINE=webnovel \
  NOVELFORGE_QA_CHAPTERS="1-20" NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf036-qa \
  timeout 900 src-tauri/target/debug/novelforge
```

Expected evidence:

| Case | Pass condition |
| --- | --- |
| Resolve | `resolved … source=https://novelphoenix.net/… confidence=1.00`; NovelFire appears only as an alternative |
| Index | `total=2334 complete=true`, 59 TOC pages, no retries, **≈42 s** |
| Crawl | 20/20 chapters, **≈56 s**, EPUB `verified=true` |
| Progress | `Chapter audit · 20/20 chapters present and non-empty` |
| Offline rank | `sources::tests::measured_cheaper_source_wins_the_tie_break` |

Fixtures: `scripts/fixtures/novelphoenix/` holds the real `.net` book page, TOC
page 2 (`ul#idData`, 40 rows), chapter 7 (`#chapter-content`) and search page,
plus the `.com` equivalents (`ul.chapter-list`, `#content`) used for the parity
check.

### Crawl watchdog and single retry (NF-035)

The watchdog itself takes 5–15 minutes to prove against a real source, so the
recipe proves it with fault-injection engines that reuse the same launch path.
`NOVELFORGE_QA_STALL_SECS` is debug-only; release builds always use the shipped
bounds.

```bash
# 1. No false stall: a healthy two-chapter run must complete untouched.
rm -rf /tmp/opencode/nf035-live && mkdir -p /tmp/opencode/nf035-live
NOVELFORGE_QA_QUERY="https://novelphoenix.net/reverend-insanity.html" \
  NOVELFORGE_QA_CHAPTERS="1-2" \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf035-live \
  timeout 900 src-tauri/target/debug/novelforge

# 2. A bad URL fails closed: exactly one launch, no retry line.
NOVELFORGE_QA_QUERY="https://example.com/not-a-novel.html" \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf035-live \
  timeout 240 src-tauri/target/debug/novelforge

# 3. Zero-output transient: two launches, one retry line.
cat > /tmp/opencode/fake-lncrawl-transient.sh <<'SH'
#!/bin/sh
n=$(cat /tmp/opencode/nf035-fake-count 2>/dev/null || echo 0); n=$((n + 1))
echo "$n" > /tmp/opencode/nf035-fake-count
echo "Exhausted: no detection layer succeeded — transport failure (Timeout)" >&2
exit 1
SH
chmod +x /tmp/opencode/fake-lncrawl-transient.sh
rm -f /tmp/opencode/nf035-fake-count
NOVELFORGE_QA_QUERY="https://example.com/x.html" \
  NOVELFORGE_QA_LNCRAWL=/tmp/opencode/fake-lncrawl-transient.sh \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf035-fake \
  timeout 120 src-tauri/target/debug/novelforge
cat /tmp/opencode/nf035-fake-count   # 2

# 4. Fatal control: same shape, but a fatal reason must launch once.
sed 's/transport failure (Timeout)/No crawler found for the domain/' \
  /tmp/opencode/fake-lncrawl-transient.sh > /tmp/opencode/fake-lncrawl-fatal.sh
chmod +x /tmp/opencode/fake-lncrawl-fatal.sh
rm -f /tmp/opencode/nf035-fake-count
NOVELFORGE_QA_QUERY="https://example.com/x.html" \
  NOVELFORGE_QA_LNCRAWL=/tmp/opencode/fake-lncrawl-fatal.sh \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf035-fake \
  timeout 120 src-tauri/target/debug/novelforge
cat /tmp/opencode/nf035-fake-count   # 1

# 5. Stall + resume: write one chapter to the lncrawl store, then hang.
cat > /tmp/opencode/fake-lncrawl-stall.sh <<'SH'
#!/bin/sh
n=$(cat /tmp/opencode/nf035-fake-count 2>/dev/null || echo 0); n=$((n + 1))
echo "$n" > /tmp/opencode/nf035-fake-count
if [ -n "$LNCRAWL_DATA_PATH" ]; then
  dir="$LNCRAWL_DATA_PATH/novels/qa/chapters"
  mkdir -p "$dir"; printf 'fake-zstd-body-%s' "$n" > "$dir/00000$n.zst"
fi
echo "fake crawl invocation $n is now silent"
exec sleep 120
SH
chmod +x /tmp/opencode/fake-lncrawl-stall.sh
rm -rf /tmp/opencode/nf035-stall && rm -f /tmp/opencode/nf035-fake-count
NOVELFORGE_QA_QUERY="https://example.com/x.html" \
  NOVELFORGE_QA_STALL_SECS=3 \
  NOVELFORGE_QA_LNCRAWL=/tmp/opencode/fake-lncrawl-stall.sh \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf035-stall \
  NOVELFORGE_QA_KEEP_WORK=1 \
  timeout 90 src-tauri/target/debug/novelforge
cat /tmp/opencode/nf035-fake-count   # 2
find /tmp/opencode/nf035-stall/.novelforge-work -name '*.zst'   # kept
```

Expected evidence:

| Case | Pass condition |
| --- | --- |
| Good run | `Completed`, `Chapter audit · 2/2`, EPUB `verified=true`, `Timing · crawl ≈45 s … retry —`, no stall/retry line |
| Bad URL | one `Launching lncrawl` line, failed in ≈2.5 s, `retry —`, message `Source error · … No crawler found for the domain` (NF-042) |
| Transient | exactly **2** invocations, `retrying once from scratch (attempt 2/2)`, final message `Process exited with code 1 · one automatic retry also failed · … Timeout` |
| Fatal | exactly **1** invocation, `retry —`, message carries the fatal reason |
| Stall | watchdog kills at 3 s, `Stalled with partial output — resuming missing chapters (attempt 2/2)`, **2** invocations, `crawl 5.4s · retry 5.4s`, message `Source stalled: … · one automatic retry also failed`, work dir kept |
| Offline | `process::stall_tests` (9): a real `sleep 30` child is killed, pause time excluded, retry kind/attempt/engine gating, `--all` → `--resume` |

Pause accounting is covered offline only: `watchdog_does_not_count_paused_time_as_silence`
asserts the watchdog does not fire while the job row is `Paused`, which is the
same condition `pause_download` sets.

### Measured pace, update ETA and faster source (NF-041)

```bash
# 1. A real full book records its own crawl cost (NovelPing, 53 chapters).
rm -rf /tmp/opencode/nf041-novelping
NOVELFORGE_QA_QUERY="https://novelping.com/book/prison-of-eternal-sky" \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf041-novelping \
  timeout 300 src-tauri/target/debug/novelforge
# expect: Success · Chapter audit 53/53 · Measured crawl pace · ~94 ch/min · 0.64 s/chapter

# The Library response exposes sourceHost/sourceRank/crawl for the card.
NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf041-novelping \
  NOVELFORGE_QA_LIBRARY=1 timeout 120 src-tauri/target/debug/novelforge

# 2. Create a real two-chapter gap, then update: the stored pace becomes the ETA.
folder=$(ls -d /tmp/opencode/nf041-novelping/*/ | head -1)
rm "$folder/chapters/0020 - "* "$folder/chapters/0021 - "*
NOVELFORGE_QA_ACTION=update NOVELFORGE_QA_FOLDER="$folder" \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf041-novelping \
  timeout 240 src-tauri/target/debug/novelforge
# expect: expected pace=Some(94.38) ch/min expected_seconds=Some(2), Completed,
# 53/53, no missing file, crawl rewritten as the 2 chapters this run crawled

# 3. Faster-host lookup never switches by itself.
slow=$(ls -d /tmp/opencode/nf041-live/reverend-insanity-*/ | head -1)
NOVELFORGE_QA_ACTION=faster NOVELFORGE_QA_FOLDER="$slow" \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf041-live \
  timeout 240 src-tauri/target/debug/novelforge
# NovelFire (rank 5) → alternative novelphoenix.net rank 0, confidence 1.00
fast=$(ls -d /tmp/opencode/nf035-live/reverend-insanity-*/ | head -1)
NOVELFORGE_QA_ACTION=faster NOVELFORGE_QA_FOLDER="$fast" \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf035-live \
  timeout 240 src-tauri/target/debug/novelforge
# NovelPhoenix (rank 0) → alternative none, “already the fastest measured host”
```

Expected evidence:

| Case | Pass condition |
| --- | --- |
| Persisted pace | `metadata.json` has `crawl.chapters` equal to the chapters this run wrote, plus seconds, s/chapter, ch/min and `measuredAt`; no pace when zero chapters |
| Library card | `list_library` has `sourceHost`, `sourceRank` (NovelPing 4, NovelFire 5, NovelPhoenix 0) and `crawl`; a folder without a measurement has `crawl: null` |
| Update ETA | `expectedChaptersPerMinute` / `expectedSeconds` are returned *and* emitted; dialog row shows `Measured 94.4 ch/min on this source · expected crawl 00:02` |
| Update pace rewrite | the new `crawl.chapters` is the selection size (2), not the merged book total (53) |
| Faster source | only a download-ready host with lower registry rank and relevance ≥ 0.85 is offered; confirming starts a `copy` download and the stored entry keeps its `sourceUrl` |
| No alternative | rank-0 host returns `alternative: none` with the “already fastest” note |

### Chapter names for partially-numbered catalogues (NF-043)

```bash
# 1. Index only. Reverend Insanity leaves chapters 1-347 as "Chapter N".
NOVELFORGE_TITLE_DEBUG=1 \
NOVELFORGE_QA_INDEX_URL="https://novelphoenix.net/reverend-insanity.html" \
  timeout 420 src-tauri/target/debug/novelforge
# expect: [titles] start: 379/2334 rows numbered, source novelphoenix.net
#         donor … novelfire.net complete=true chapters=2334
#         [qa] title source: novelfire.net
#         [qa] chapter 1 "The heart of a demon never has regret even in death"
#         [qa] chapter 1168 "Chapter 1168 - Lightning Refinement, Ma Hong Yun" (source name kept)

# 2. Omniscient Reader's Viewpoint: NovelFire has the same 65 gaps, so the names
#    are recovered from NovelPhoenix chapter pages (Episode/Prologue headings).
NOVELFORGE_TITLE_DEBUG=1 \
NOVELFORGE_QA_INDEX_URL="https://novelphoenix.net/omniscient-readers-viewpoint.html" \
  timeout 420 src-tauri/target/debug/novelforge
# expect: 65/551 rows numbered → 0 exact donor candidates →
#         Recovered 65 chapter name(s) from this source's chapter pages
#         chapter 0 "There are three ways to survive in a ruined world"
#         chapter 276 "■■ (1)"

# 3. Lord of the Mysteries must stay identical (one bare row, then recovery).
NOVELFORGE_QA_INDEX_URL="https://novelphoenix.net/lord-of-the-mysteries.html" \
  timeout 240 src-tauri/target/debug/novelforge
# expect: chapter 1 "Chapter 1 - Crimson", no "title source" line, 0 bare rows

# 4. The bundled EPUB carries the same names: nav, NCX, <title> and <h1>.
NOVELFORGE_QA_QUERY="https://novelphoenix.net/reverend-insanity.html" \
NOVELFORGE_QA_CHAPTERS="1-2" NOVELFORGE_QA_DUPLICATE="copy" \
  timeout 300 src-tauri/target/debug/novelforge
cdn=$(ls -dt ~/Downloads/NovelForge/reverend-insanity-* | head -1)
unzip -p "$cdn/Reverend Insanity.epub" EPUB/nav.xhtml | grep chapter_00001
unzip -p "$cdn/Reverend Insanity.epub" EPUB/chapter_00001.xhtml | grep -E "<title>|<h1>"
# expect: "Chapter 1 - The heart of a demon never has regret even in death"
```

Expected evidence:

| Case | Pass condition |
| --- | --- |
| RI donor | `title source: novelfire.net`, 379 names filled, chapter 1/2 named, chapter 1168 keeps its source name |
| ORV recovery | 65 names recovered from chapter pages, chapter 0 and 276 named, 0 bare rows, no `title source` line |
| LOTM | unchanged source names, 1 bare row recovered, no caption |
| EPUB | nav/NCX/`<title>`/`<h1>` show `Chapter N - <name>`, `verified.ok=true`, `complete=true` |
| Fail closed | an index with too few donor names (coverage < 95 %) or a shifted donor changes nothing and keeps the source's numeric titles |

### Visible chapter numbers vs lncrawl serials (NF-044)

```bash
# 1. ORV starts at visible Chapter 0: visible 2-3 are lncrawl serials 3-4.
NOVELFORGE_QA_QUERY="https://novelphoenix.net/omniscient-readers-viewpoint.html" \
NOVELFORGE_QA_CHAPTERS="2-3" NOVELFORGE_QA_DUPLICATE="copy" \
  timeout 600 src-tauri/target/debug/novelforge
# expect: "Chapter audit · 2/2 chapters present and non-empty", exactly
#   chapters/0002 - Starting the Paid Service (2).xhtml  (<h1>Chapter 2</h1>)
#   chapters/0003 - Starting the Paid Service (3).xhtml  (<h1>Chapter 3</h1>)
# no stray 0004 (the crawler's provisional serial-4 file must be swept), and
# EPUB toc.ncx labels Chapter 2 / Chapter 3.

# 2. A range that mixes named and unnamed rows.
NOVELFORGE_QA_QUERY="https://novelphoenix.net/omniscient-readers-viewpoint.html" \
NOVELFORGE_QA_CHAPTERS="275-277" NOVELFORGE_QA_DUPLICATE="copy" \
  timeout 600 src-tauri/target/debug/novelforge
# expect 3/3: 0275 - Chapter 275 - Giant Story (4).xhtml, 0276 - (1).xhtml,
# 0277 - (2).xhtml; EPUB nav Chapter 275 / 276 / 277.

# 3. The source's TOC has no visible Chapter 111: fail before touching disk.
before=$(ls -d ~/Downloads/NovelForge/*omniscient* | wc -l)
NOVELFORGE_QA_QUERY="https://novelphoenix.net/omniscient-readers-viewpoint.html" \
NOVELFORGE_QA_CHAPTERS="111" NOVELFORGE_QA_DUPLICATE="copy" \
  timeout 300 src-tauri/target/debug/novelforge
# expect: "Chapter 111 is not in the source's chapter list, so the download was
# not started. The list shows 551 chapter(s)." and the folder count unchanged.

# 4. A 1-based source maps to itself (no regression).
NOVELFORGE_QA_QUERY="https://novelphoenix.net/reverend-insanity.html" \
NOVELFORGE_QA_CHAPTERS="1-2" NOVELFORGE_QA_DUPLICATE="copy" \
  timeout 600 src-tauri/target/debug/novelforge
# expect 2/2: 0001 - The heart of a demon never has regret even in death.xhtml,
# 0002 - Going back in time with 500 years of knowledge.xhtml.
```

Evidence logs: `/tmp/opencode/nf044-orv-2-3c.log`,
`/tmp/opencode/nf044-orv-275.log`, `/tmp/opencode/nf044-orv-111.log`,
`/tmp/opencode/nf044-ri-1-2.log`.

### Fatal crawler reason on stdout (NF-042)

```bash
# 1. Real lncrawl trigger: error on stdout, exit 0, no artifact.
rm -rf /tmp/opencode/nf042-live
NOVELFORGE_QA_QUERY="https://example.com/not-a-novel.html" \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf042-live \
  timeout 180 src-tauri/target/debug/novelforge
# expect: 1 launch · crawl ≈2.4 s · message
# Source error · lncrawl._server_errors.ServerError: Error(501): No crawler found for the domain

# 2. Same shape, fault-injected on stdout with exit 0.
cat > /tmp/opencode/fake-lncrawl-stdout-fatal.sh <<'SH'
#!/bin/sh
n=$(cat /tmp/opencode/nf042-fake-count 2>/dev/null || echo 0); n=$((n + 1))
echo "$n" > /tmp/opencode/nf042-fake-count
echo "Traceback (most recent call last):"
echo '  File "/tmp/lncrawl/__main__.py", line 40, in <module>'
echo "    run()"
echo "lncrawl._server_errors.ServerError: Error(501): No crawler found for the domain"
exit 0
SH
chmod +x /tmp/opencode/fake-lncrawl-stdout-fatal.sh
rm -f /tmp/opencode/nf042-fake-count
NOVELFORGE_QA_QUERY="https://example.com/not-a-novel.html" \
  NOVELFORGE_QA_LNCRAWL=/tmp/opencode/fake-lncrawl-stdout-fatal.sh \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf042-stdout \
  timeout 120 src-tauri/target/debug/novelforge
cat /tmp/opencode/nf042-fake-count   # 1

# 3. Transient control still retries exactly twice (stderr stub from NF-035).
rm -f /tmp/opencode/nf035-fake-count
NOVELFORGE_QA_QUERY="https://example.com/not-a-novel.html" \
  NOVELFORGE_QA_LNCRAWL=/tmp/opencode/fake-lncrawl-transient.sh \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf042-transient \
  timeout 120 src-tauri/target/debug/novelforge
cat /tmp/opencode/nf035-fake-count   # 2
```

Expected evidence:

| Case | Pass condition |
| --- | --- |
| Real bad URL | **1** lncrawl launch, `retry —`, message `Source error · … No crawler found for the domain`, no “no usable output file” headline |
| stdout exit 0 | traceback frames and INFO rows are ignored; the final `ServerError` line is the message; **1** launch; `is_transient()` false |
| Transient stderr | **2** launches and `one automatic retry also failed · … Timeout` (no regression from feeding stdout) |
| Fatal stderr | **1** launch and the fatal reason in the message |

### Live chapter structuring during a crawl (NF-039)

```bash
# 40 chapters is enough to watch files appear mid-crawl.
rm -rf /tmp/opencode/nf039
NOVELFORGE_QA_QUERY="https://novelphoenix.net/reverend-insanity.html" \
  NOVELFORGE_QA_ENGINE=webnovel NOVELFORGE_QA_CHAPTERS="1-40" \
  NOVELFORGE_QA_DOWNLOAD_DIR=/tmp/opencode/nf039 \
  timeout 300 src-tauri/target/debug/novelforge > /tmp/opencode/nf039.log 2>&1 &
# While it runs: watch the destination folder grow.
watch -n 2 'ls /tmp/opencode/nf039/*/chapters | wc -l; grep -o "phase=[A-Za-z]*" /tmp/opencode/nf039.log | tail -1'
```

Expected evidence:

| Case | Pass condition |
| --- | --- |
| During crawl | `chapters/00NN - ….xhtml` exists while the phase is still `DownloadingChapters`, growing with the `.zst` count |
| Log | `Live structuring · N chapter file(s) ready while the crawl continues` |
| Join | `Live structuring · N chapter file(s) ready before the final export` |
| Export | `Timing · … export 0.4s`, `Completed`, `Chapter audit · 40/40` |
| Cleanup | exactly 40 chapter files + `style.css`; no provisional leftovers, no `*.part`; `xmllint --noout` accepts each file |

Unit coverage: `structurer::tests` (real saved `.zst` fixture, mid-write retry,
sweep safety) and `chapter_clean::tests::void_elements_are_self_closing_so_the_xhtml_parses`.

### Source measurements and budgets (NF-030)

```bash
# Same five titles × every source, real downloads, JSON + Markdown evidence
~/.local/share/novelforge/tools/aget-venv/bin/python scripts/measure_sources.py \
  --budget 120

# Single source / title while debugging
~/.local/share/novelforge/tools/aget-venv/bin/python scripts/measure_sources.py \
  --budget 60 --sources ocean --titles "Fourth Wing|Rebecca Yarros"
```

Expected evidence:

| Case | Pass condition |
| --- | --- |
| Fast-fail | a blocked source prints one `… after N.Ns — skipped` line and the next source starts; no polling past the budget |
| WeLib | both mirrors are checked; `HTTP 403`/`CAPTCHA` reported in ~2–3 s, not retried for minutes |
| Anna's | a verified EPUB (measured 40.9–89.3 s) or one bounded skip line at the budget |
| Ocean headless | headless Chromium either produces a verified EPUB or prints `Cloudflare/Turnstile challenge … — skipped` within the budget; no visible window |
| LibGen | author-augmented search finds exact editions (`Pride and Prejudice: Jane Austen`) that a bare title misses; decoys (omnibus, anthology, non-English) fail closed |
| Output | `docs/measurements/nf-030-<date>.md` records success, seconds, size, format and the deciding log line per row |

Measured baseline (2026-11-16, budget 150 s): LibGen 3/5 (9.4–17.6 s; a fourth
matched but its file host stalled), WeLib 0/5 (2.9–25.6 s, blocked), Anna's 4/5
(38.6–80.0 s), Ocean 0/5 (bounded at 148.7–150.7 s, Turnstile). Use `--merge` to combine a rerun of a single row with
the existing JSON instead of discarding the rest:

## 3. Merge gate for a new source

A new source profile or adapter is not mergeable until all four exist:

1. **Registry entry** in `src-tauri/src/sources.rs` with `evidence` and a
   `live_matrix` date whenever `support` is download-ready.
2. **Offline parser fixture** for the source's distinctive markup (TOC shape,
   AJAX envelope, metadata selectors) in the module's `#[cfg(test)]` tests.
3. **Smoke result** from `scripts/source-smoke.sh <target>` attached to the
   change, with the log path and the index count.
4. **Failure evidence** for the unsafe path: a truncated TOC, a blocked/429
   response, or an empty archive must fail closed.

`source-smoke.sh` and the parser fixtures are additive; deleting an existing
target or fixture requires a matching backlog entry explaining why.

## 4. Desktop layout matrix (manual)

Re-check after any change to the shell, page padding or type scale:

| Viewport | Pages checked | Expected |
|---|---|---|
| 1440×900 (full width) | Home, About, Guardrails, Library, Books | right rail visible at 340 px, About pipeline and Guardrails stats in 4 columns, no clipped text |
| 900×620 (Tauri minimum) | Home, About, Guardrails, Library, Books | sidebar collapses to the 74 px icon rail, stats in 2 columns, Library table scrolls horizontally, content keeps a clear gap from the rail |

Both sizes were checked on 2026-11-16 after the UI pass. The window is a Wayland
client, so the check is visual (screenshot) rather than DOM-measured; the
offline gate still covers behaviour and the production bundle.
