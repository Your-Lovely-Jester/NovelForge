# NovelForge — Release Readiness

**Last updated:** 2026-11-17  
**Applies to:** `0.1.0` binary-only release  
**Companion:** `docs/SOURCE_PIPELINE_BACKLOG.md` (NF-045 … NF-061), `docs/QA_MATRIX.md`

---

## 1. Decision and scope

NovelForge will be published as a **compiled installer**, not as a source
repository. The source tree stays private. This changes three things:

1. Copyrighted test fixtures are no longer a **publication** blocker, but they
   still must not be bundled into an installer, and they must not be copied into
   a public release artifact of any kind.
2. The GPL-derived lncrawl patches are still distributed inside the build
   inputs. Binary-only distribution does **not** remove the GPL-3.0 obligation
   attached to those derivative works (see §6).
3. “Open source” must not be claimed anywhere in the README, About page, site
   or store listing. The correct description is “freeware / source-available
   on request” only if that is actually true.

The release target is a **Linux alpha first**. Windows is validated next, on a
real Windows machine or CI runner. Android is a separate product decision, not
a port task (§5).

---

## 2. Status after the release-readiness batch

| Item | Status | Evidence |
|---|---|---|
| NF-045 atomic durable writes | **DONE** | `src-tauri/src/fsutil.rs`; unit tests `atomic_write_replaces_content_and_leaves_no_temp`, `a_failed_replace_removes_its_temporary` |
| NF-046 quit guard | **DONE** | `lib.rs` `CloseRequested` + `ExitRequested`, `EVENT_QUIT_REQUESTED`, `confirm_quit`, React modal |
| NF-047 job journal / interrupted resume | **DONE** | `AppState::persist_journal` / `restore_journal`, `TaskStatus::Interrupted`, `resume_interrupted`; tests `journal_tests` (3) |
| NF-048 persistent log + diagnostics | **DONE** | `emit_log` → `novelforge.log`, 5 MiB rotation, `get_diagnostics`, About → **Copy diagnostics** |
| NF-049 error boundary | **DONE** | `src/components/ErrorBoundary.tsx`, mounted in `main.tsx` |
| NF-050 single instance | **DONE** | `tauri-plugin-single-instance`, focuses the existing window |
| NF-051 Windows process control | **DONE** | `terminate_process` / `force_kill_process`; `taskkill /PID /T [/F]`; pause/resume fail closed off Unix |
| NF-052 pinned/modular toolchain | **DONE** | `scripts/install-tools.sh`: pinned versions + `webnovel`/`manga`/`books`/`all` |
| NF-053 fmt + clippy gate | **DONE** | `cargo fmt --check` clean; clippy clean; wired into `check.sh` and CI |
| NF-054 ZIP asset path hardening | **DONE** | `export::safe_entry_path` |
| NF-055 schema version | **DONE** | `model::SCHEMA_VERSION` stamped on write |
| NF-056 corrupt catalogue no longer aborts | **DONE** | `catalogue::entries` logs to stderr and returns empty |
| NF-057 Windows runtime validation | **OPEN** | §4; Wine cannot test WebView2 |
| NF-058 Android | **OPEN (spike)** | §5; not a port |
| NF-059 binary-only packaging / signing / GPL | **OPEN** | §6 |
| NF-060 strip personal modifications | **DONE** | `scripts/check-personal-data.sh` (in `check.sh`) blocks `/home/<user>/` paths, bundle resources, non-`resources/` embeds and fixture markers; docs paths sanitised; `.gitattributes` export-ignores fixtures/measurements; see §7 |
| NF-061 dead-code, redundancy and unused-file audit | **DONE** | `knip` (in `check.sh` + CI) / `cargo-machete` / `ruff` / `vulture` / `clippy`; removed `HeroArt.tsx`, 6 unused helpers, 11 unexported types, 8 CSS classes, 4 `cover-*.jpg`, 7 unused fixtures, `sources::for_url`, `ManifestItem.media_type`; deduped `civil_from_days`, `key_hash`, `is_hidden_path`, `hostFromUrl`, test `novel()`; see `docs/QA_MATRIX.md` §1 |
| NF-024 public binary + donation channel | **OPEN** | Blocked by NF-057/059; direction already binary-only |

Gate after the batch: **201 Rust lib tests + 2 integration + 36 frontend + 38
Python**, clippy clean, `cargo fmt --check` clean, `knip` clean, production build
green, and the personal-data/fixture scan clean.

---

## 3. What changed, and why it matters

### 3.1 Durable writes (NF-045)

Before: `metadata.json`, `index.json`, `chapters.html` and `settings.json` were
written in place. A crash, full disk or power loss mid-write could leave a
finished download with unreadable metadata (orphaned in the Library) or wipe
the user's settings.

After: every durable file goes through `fsutil::write_atomic` — write to a
sibling temp file, `sync_all`, then replace. Unix uses `rename`; Windows uses
`MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`. The retitled
EPUB streams to `sibling_temp` and replaces the original only after the write
completes. The job journal and the index cache use the same helper.

### 3.2 Quit guard and interruption record (NF-046 / NF-047)

Before: `window.close()` called `shutdown_all` immediately. A public user could
lose an hours-long crawl by clicking X, and the next launch had no record of it.

After:

- `CloseRequested` and `ExitRequested` are prevented while any job is non-terminal.
- The backend emits `app://quit-requested`; the frontend shows a modal that says
  partial files stay on disk and can be resumed.
- **Stop and quit** calls `confirm_quit`, which sets `allow_exit`, marks active
  jobs `Interrupted`, writes `jobs.json`, then sends `SIGTERM`/`SIGKILL` (Unix)
  or `taskkill /T /F` (Windows).
- On boot, `restore_journal` reloads `jobs.json`; active statuses become
  `Interrupted`, terminal rows keep their history.
- The Downloads row for an interrupted job has a Resume button:
  `resume_interrupted` → `library_actions::retry_download`, so chapter selection,
  index verification and the fail-closed audit still apply.

This is deliberately **not** a claim of true suspend/resume on Windows. Pause
and resume are Unix process signals; on Windows they now return an explicit
error instead of pretending. See §4.

### 3.3 Persistent log and diagnostics (NF-048)

`novelforge.log` sits beside `settings.json`
(`~/.config/com.novelforge.app/novelforge.log` on Linux). Rows are UTC with
millisecond precision, rotate at 5 MiB to `.log.1`, and include job id and
stream. About → **Copy diagnostics** returns a plain-text report containing:

- version, Tauri version, OS, architecture;
- download root, config file, log path;
- a redacted settings summary (never the Hardcover token);
- each tool's found/version/path/detail;
- active jobs with status and progress;
- total and interrupted task counts;
- the last 200 log lines.

Home-directory prefixes are replaced with `~`. Page text, chapter text and
tokens are never included.

### 3.4 Single instance and error boundary (NF-049 / NF-050)

A second launch now raises the existing window instead of racing
`settings.json` and `jobs.json`. A React render error shows a recoverable screen
with the message, a Reload button and Copy error, instead of a white window.

### 3.5 Process control and toolchain (NF-051 / NF-052)

- Unix: `SIGCONT` + `SIGTERM` to the process group, then `SIGKILL` after 250 ms.
- Windows: `taskkill /PID <pid> /T` then `/F`; `/T` is required because the
  venv launcher owns the Python grandchild.
- Pause/resume are Unix-only and now fail closed elsewhere.
- `install-tools.sh` pins `gallery-dl 1.32.20`, `click 8.5.1`,
  `nodriver 0.50.99`, `playwright 1.65.0`, `rich 15.0.0`, and already pinned
  lncrawl `v4.14.0`. Components can be installed separately. Native Windows
  exits with an explicit message instead of half-installing bash wrappers.

---

## 4. Windows validation plan

### 4.1 Why Wine is not a valid test

Tauri 2 on Windows renders with **WebView2** (Microsoft Edge Runtime). Wine does
not provide WebView2. A Wine run can start the binary but cannot prove the UI,
IPC, custom titlebar, file dialogs or window-close guard work. Do not record a
Wine run as Windows validation.

### 4.2 Options, in order of preference

1. **Windows VM** (QEMU/VirtualBox/Hyper-V, or a cloud Windows instance).
   - Install the WebView2 Evergreen Runtime if the image lacks it.
   - Install the three engines natively:
     `py -3 -m venv`, `pip install -e` lncrawl, `pip install gallery-dl==1.32.20`,
     and the pinned aget deps. The bundled Windows installer must not depend on
     the bash script.
   - Run the acceptance list below.
2. **GitHub Actions `windows-latest`** for build, compile checks and packaging.
   `.github/workflows/windows-build.yml` runs `cargo fmt --check`,
   `cargo clippy -D warnings` (which compiles the `#[cfg(windows)]` branches),
   `cargo test`, then `tauri build --bundles nsis` and uploads the `.exe` plus
   `SHA256SUMS.txt` as the `novelforge-windows-nsis` artifact. Trigger it with
   **Actions → windows-build → Run workflow** or by pushing a `v*` tag. It needs
   no Windows ISO, no local VM and about 30 MB of transfer (source up, installer
   down).
3. **A physical Windows machine**, if one is available, for the final
   installer test.

### 4.3 Windows acceptance list

- [ ] Installer runs without admin rights and creates a Start Menu entry.
- [ ] First launch opens, no blank WebView2 window.
- [ ] Settings path is under `%APPDATA%\com.novelforge.app` (or Tauri's
      resolved config dir), not a Linux path.
- [ ] Tool discovery finds native `lncrawl.cmd` / `gallery-dl.exe` / `aget`.
- [ ] A short webnovel download (2 chapters) produces cover, `metadata.json`,
      `chapters/`, `index.json`, `chapters.html` and one verified EPUB.
- [ ] Cancel kills the Python grandchild (`tasklist` shows no orphan).
- [ ] Closing the window during a crawl shows the quit modal.
- [ ] **Stop and quit** marks the job interrupted; relaunch restores the row; Resume
      works.
- [ ] Pause shows an explicit unsupported-platform error, not a fake pause.
- [ ] Long paths: download root near `MAX_PATH` still writes chapter files.
- [ ] `cargo fmt --check` and `clippy -D warnings` pass on Windows.
- [ ] Single instance: second launch focuses the first window.

Any failure becomes an NF item in `SOURCE_PIPELINE_BACKLOG.md`; do not mark
NF-057 DONE on compile-only evidence.

### 4.4 Modified Windows images (LTSC / Atlas OS / debloated builds)

A friend's Windows 10 LTSC or Atlas OS machine is a useful **best-effort smoke
test**, not a stock-Windows validation run. Modified images usually remove Edge,
which also removes the **WebView2** runtime the Tauri window needs; some disable
the Windows Installer service or Windows Update.

- Ship the **NSIS `.exe`**, not the MSI. NSIS does not need the Windows Installer
  service.
- If the app opens blank or shows a WebView2 error, install the **WebView2
  Evergreen Runtime** (Microsoft's standalone installer, ~150 MB) and relaunch.
  The Tauri installer's default `downloadBootstrapper` mode will also try to
  fetch it during setup when internet is available.
- The installer is unsigned, so SmartScreen shows "Windows protected your PC";
  the tester must use **More info → Run anyway**.
- Ask for `winver` output and whether WebView2 was already present, so a failure
  can be attributed to the image rather than the app.
- Items that need no Python engines and can be checked immediately: install,
  first launch, Settings paths, About diagnostics, single instance, quit modal,
  library UI, theme/DPI. The crawl items will fail until the native Windows tool
  bootstrap exists (NF-057), so those failures are expected findings, not
  regressions.

Record the result as "smoke test on a modified Windows 10 image"; only a stock
Windows 10/11 VM or physical machine closes NF-057.

---

## 5. Android: why this is a spike, not a port

NovelForge is a **process orchestrator**. Every engine is a Python program with
a virtualenv and, for books, a Chromium driven by Playwright:

| Engine | Runtime requirement |
|---|---|
| lncrawl | Python 3 + editable package + writable data dir |
| gallery-dl | Python 3 + per-site extractors |
| aget | Python 3 + Playwright Chromium + `nodriver` |

Android applications cannot fork arbitrary Python executables from assets;
`ProcessBuilder` is not available to normal apps, and Playwright Chromium is not
shippable through the Play Store in that form. A Tauri mobile build would also
need a WebView-compatible shell and a rewritten storage layer.

Options to evaluate in a time-boxed spike:

1. **Desktop/server companion.** Android app is a thin client to a NovelForge
   instance on the user's LAN. Lowest backend risk; requires auth, pairing and
   an HTTP API that does not exist today.
2. **Native rewrite.** Reimplement crawlers in Rust and package them in the APK.
   Large effort; loses lncrawl's 300+ adapters and gallery-dl's catalogue.
3. **Termux-style bundle.** Ship an APK that depends on Termux packages. Fragile,
   Play-Store hostile, and not a normal user experience.

Recommendation: do not attempt Android for `0.1.0`. Write the spike as a
separate document when the desktop release is stable.

---

## 6. Binary-only packaging, licensing and signing

### 6.1 What ships

- Linux: AppImage and/or `.deb` from `tauri build`.
- Windows: NSIS `.exe` and MSI, unsigned until a certificate exists.
- macOS: not in scope for the alpha.

Each artifact gets a `SHA256SUMS.txt`. The release notes state the exact commit,
tool versions and whether the build is unsigned.

### 6.2 Third-party obligations

| Component | License | Obligation |
|---|---|---|
| lncrawl / lightnovel-crawler | GPL-3.0 | Source offer for the patched work; the patches are derivative |
| lncrawl-scraper patch | GPL-3.0-derived | Same as above |
| gallery-dl | GPL-2.0-only | License text + source offer if redistributed |
| aget (upstream) | **no LICENSE file found** | Do not redistribute the checkout; install from upstream at the user's request |
| Tauri / Rust crates | MIT/Apache-2.0 mostly | `THIRD_PARTY_NOTICES.md` generated from `cargo metadata` |
| npm dependencies | per package | `THIRD_PARTY_NOTICES.md` from `npm ls --json` |

The safe binary-only shape:

1. Do **not** bundle lncrawl/gallery-dl/aget checkouts in the installer.
2. Ship a first-run installer that fetches them from upstream, applies the
   repository-owned patches locally, and records the exact commits.
3. Publish `scripts/patches/*` and the patched-file diffs under **GPL-3.0** in
   a small public `novelforge-patches` repository (or a `patches/` directory in
   the release assets), with a written offer for the complete corresponding
   source of the patched lncrawl work.
4. Keep `aget` out of the default install and out of the bundled artifacts until
   its license is clarified upstream.
5. Add `THIRD_PARTY_NOTICES.md` and show it in About.

This is packaging guidance, not legal advice. Have the GPL split reviewed once
before the first public binary.

### 6.3 Signing and updates

- Windows: Authenticode certificate or an explicit “unsigned, SmartScreen will
  warn” note. Do not silently ship unsigned and blame the user.
- Linux: publish the AppImage with a detached signature or at minimum SHA-256.
- Updates: no auto-updater in `0.1.0`. A manual “check for updates” link is
  enough until the signing key exists.

---

## 7. Personal-modification strip list (NF-060)

These are the known local-only changes and artifacts that must not reach a
release artifact or a public repository.

**Status 2026-11-17:** the path scrub and the packaging guard are done. The
saved-page fixtures stay in the private tree because the offline tests must use
real pages; `scripts/check-personal-data.sh` proves they cannot be embedded in
the binary or declared as bundle resources. Personal absolute paths in the
backlog and measurement notes were rewritten to `~`, `scripts/__pycache__` was
removed and ignored, and `.gitattributes` export-ignores the fixture and
measurement paths for any `git archive`.

### 7.1 Never ship (they may stay in the private dev tree)

| Path | Why |
|---|---|
| `src-tauri/tests/fixtures/novelphoenix/chapter_0001.zst` | 655-byte chapter excerpt |
| `scripts/fixtures/libgen/*.html` | Pages captured from a piracy mirror |
| Any `scripts/fixtures/**` page containing live site chrome/ads | Not needed for a binary |
| `/tmp/opencode/nf044-*.log`, QA logs, downloaded test libraries | Personal test artifacts |
| `~/Downloads/NovelForge/*` used during QA | Never package a user library |

The seven unused `scripts/fixtures/novelphoenix/*.html` pages (including
`net-chapter-7.html` and `com-chapter-100.html`) were deleted on 2026-11-17:
no test or script referenced them. The surviving fixtures are the ones the
offline suites actually read (`libgen`, `novelfire`, `gutenberg`,
`standardebooks`), plus the two chapter excerpts embedded by Rust tests.

The public-domain `src-tauri/tests/fixtures/tell_tale_heart_chapter_0001.xhtml`
may stay, but it is test-only and should not be in the installer either.

### 7.2 Scrub from tracked files before handing the tree to a build

- Hard-coded `/home/<user>/...` paths (search the whole tree, including
  docs and scripts).
- Personal download roots in `settings.json` examples and screenshots.
- Personal source-limit overrides measured on this machine.
- Any `NOVELFORGE_QA_*` environment values baked into scripts or docs.
- Temporary `-copy-N` folders, `dist/`, `src-tauri/target/`, `node_modules/`.
- The 307 KB `SOURCE_PIPELINE_BACKLOG.md` if the tree is ever published: it
  documents per-source anti-bot workarounds and is poor public optics even
  though the engineering is sound. Keep it private or publish a curated
  `CHANGELOG.md` instead.

### 7.3 Keep, because they are product code

- `scripts/patches/*.patch` and `scripts/apply-lncrawl-patches.sh` (GPL-covered,
  see §6).
- `scripts/patch_lncrawl_scraper.py` and its tests.
- `scripts/aget.py` (NovelForge's verified LibGen-first variant).
- `scripts/install-tools.sh` (now pinned and component-selectable).

### 7.4 Suggested build-exclude

Add a `.gitattributes export-ignore` / packaging exclude for:

```text
scripts/fixtures/
src-tauri/tests/fixtures/
docs/SOURCE_PIPELINE_BACKLOG.md
.github/
node_modules/
dist/
src-tauri/target/
```

If the source is ever opened, replace prose fixtures with synthetic markup that
keeps selectors, links and structure, and move real-page tests behind an opt-in
`fetch-fixtures.sh`.

---

## 8. Release checklist

### Before building

- [ ] `./scripts/check.sh` green.
- [ ] `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] No `TODO`/`FIXME` in release paths; `grep` clean.
- [ ] Version bumped in `package.json`, `src-tauri/Cargo.toml`,
      `src-tauri/tauri.conf.json`.
- [ ] README leads with a screenshot/GIF, supported platforms, the three-command
      path and an honest “what this does not do”.
- [ ] `LICENSE`, `THIRD_PARTY_NOTICES.md`, `SECURITY.md`, `CHANGELOG.md`,
      `FUNDING.yml` present.
- [ ] Donation link is a plain “supports maintenance” link, never feature-gated.
- [x] Personal-modification strip list (§7) complete.

### Build

- [ ] `tauri build` on a clean Linux VM.
- [ ] `tauri build` on Windows (VM or CI artifact) and smoke-test §4.3.
- [ ] SHA-256 for every artifact.
- [ ] Install the artifact on a clean VM, not the development machine.

### Publish

- [ ] Release notes list version, commit, tool versions and known limitations.
- [ ] State “unsigned build” if applicable.
- [ ] State “no auto-update yet”.
- [ ] Open issues for NF-024, NF-057, NF-058, NF-059.

---

## 9. Verification commands

```bash
# Full offline gate
./scripts/check.sh

# Individual gates
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm test
npm run build

# Toolchain, one component at a time
./scripts/install-tools.sh webnovel
./scripts/install-tools.sh manga
./scripts/install-tools.sh books
```

Live QA remains opt-in and is documented in `docs/QA_MATRIX.md`
(`NOVELFORGE_ALLOW_LIVE=1 ./scripts/source-smoke.sh`).
