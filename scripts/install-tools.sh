#!/usr/bin/env bash
# NovelForge toolchain installer
# ---------------------------------------------------------------------------
# Installs the three download engines NovelForge drives:
#
#   lncrawl    webnovels  -> EPUB
#   gallery-dl manga      -> CBZ / ZIP
#   aget       e-books    -> EPUB / PDF (Anna's Archive, Playwright)
#
# Everything lands in user-owned locations, no root required:
#   ~/.local/bin/lncrawl                   (wrapper)
#   ~/.local/share/novelforge/tools/lncrawl-venv
#   ~/.local/share/novelforge/tools/lightnovel-crawler
#   ~/.local/bin/gallery-dl                (wrapper)
#   ~/.local/share/novelforge/tools/gallery-dl-venv
#   ~/.local/bin/aget                      (wrapper)
#   ~/.local/share/novelforge/tools/annas-archive-downloader
#   ~/.local/share/novelforge/tools/aget-venv
#
# Re-running is safe: existing files are replaced, the aget checkout is updated.
#
# Usage:
#   ./scripts/install-tools.sh              # all three engines
#   ./scripts/install-tools.sh webnovel     # lncrawl only
#   ./scripts/install-tools.sh manga books  # gallery-dl + aget
#
# This script targets Linux and macOS. Windows uses a separate bundle and is
# not supported by the bash wrappers below.
# ---------------------------------------------------------------------------
set -euo pipefail

WANT_WEBNOVEL=0
WANT_MANGA=0
WANT_BOOKS=0
if [ "$#" -eq 0 ]; then
  WANT_WEBNOVEL=1; WANT_MANGA=1; WANT_BOOKS=1
fi
for arg in "$@"; do
  case "${arg}" in
    webnovel|lncrawl) WANT_WEBNOVEL=1 ;;
    manga|gallery-dl) WANT_MANGA=1 ;;
    books|aget) WANT_BOOKS=1 ;;
    all) WANT_WEBNOVEL=1; WANT_MANGA=1; WANT_BOOKS=1 ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "Unknown component: ${arg} (expected webnovel, manga, books or all)" >&2; exit 2 ;;
  esac
done

# Pinned versions: an upstream release must not silently change crawl output.
# Bump these together with a live smoke run (see docs/QA_MATRIX.md).
GALLERY_DL_VERSION="1.32.20"
AGET_CLICK="8.5.1"
AGET_NODRIVER="0.50.99"
AGET_PLAYWRIGHT="1.65.0"
AGET_RICH="15.0.0"

BIN_DIR="${HOME}/.local/bin"
DATA_DIR="${HOME}/.local/share/novelforge"
TOOLS_DIR="${DATA_DIR}/tools"
GALLERY_VENV="${TOOLS_DIR}/gallery-dl-venv"
AGET_DIR="${TOOLS_DIR}/annas-archive-downloader"
AGET_VENV="${TOOLS_DIR}/aget-venv"

LNCRAWL_REPO="https://github.com/lncrawl/lightnovel-crawler.git"
# The eight patches in scripts/patches/ were generated against this commit on
# `dev`, which still reports 4.14.0 but already contains the
# `create_crawler_info` refactor that the source-capabilities patch depends on.
# Tag `v4.14.0` is older and the last three patches do not apply to it.
LNCRAWL_COMMIT="59b0382d51927953aa8120c5de62dab23ce3f731"
LNCRAWL_DIR="${TOOLS_DIR}/lightnovel-crawler"
LNCRAWL_VENV="${TOOLS_DIR}/lncrawl-venv"
AGET_REPO="https://github.com/ALBEDO-TABAI/annas-archive-downloader.git"

say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[!]\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31m[x]\033[0m %s\n' "$*" >&2; exit 1; }

need() {
  command -v "$1" >/dev/null 2>&1 || fail "Missing required command: $1"
}

case "$(uname -s 2>/dev/null || echo unknown)" in
  Linux|Darwin) ;;
  MINGW*|MSYS*|CYGWIN*)
    fail "Native Windows is not supported by this bash installer. Use the bundled Windows build or WSL." ;;
  *) warn "Unrecognised platform; continuing as Unix" ;;
esac

need curl
need python3
if [ "${WANT_WEBNOVEL}" -eq 1 ] || [ "${WANT_BOOKS}" -eq 1 ]; then
  need git
fi

mkdir -p "${BIN_DIR}" "${DATA_DIR}/bin" "${TOOLS_DIR}"

if [ "${WANT_WEBNOVEL}" -eq 1 ]; then
# --- lncrawl ---------------------------------------------------------------
# Source install, because the release binary cannot be patched and lncrawl's
# NovelFull adapter is stale (NF-019). The patch is applied below.
say "Fetching lncrawl ${LNCRAWL_COMMIT:0:10}"
need git
if [ ! -d "${LNCRAWL_DIR}/.git" ]; then
  git init -q "${LNCRAWL_DIR}"
  git -C "${LNCRAWL_DIR}" remote add origin "${LNCRAWL_REPO}"
  git -C "${LNCRAWL_DIR}" -c core.autocrlf=false fetch --depth 1 origin "${LNCRAWL_COMMIT}"
  git -C "${LNCRAWL_DIR}" checkout -q FETCH_HEAD
else
  git -C "${LNCRAWL_DIR}" config core.autocrlf false
  current="$(git -C "${LNCRAWL_DIR}" rev-parse HEAD 2>/dev/null || true)"
  if [ "${current}" != "${LNCRAWL_COMMIT}" ]; then
    say "Moving lncrawl checkout from ${current:0:10} to ${LNCRAWL_COMMIT:0:10}"
    git -C "${LNCRAWL_DIR}" -c core.autocrlf=false fetch --depth 1 origin "${LNCRAWL_COMMIT}"
    # The tree only ever holds upstream code plus our patches, which are
    # re-applied below; discarding local edits is safe and fixes a checkout
    # made with autocrlf enabled.
    git -C "${LNCRAWL_DIR}" reset -q --hard FETCH_HEAD
  else
    say "Keeping existing lncrawl checkout at ${LNCRAWL_DIR}"
  fi
fi

if [ ! -x "${LNCRAWL_VENV}/bin/python" ]; then
  python3 -m venv "${LNCRAWL_VENV}"
fi
"${LNCRAWL_VENV}/bin/python" -m pip install --upgrade pip >/dev/null
"${LNCRAWL_VENV}/bin/python" -m pip install --editable "${LNCRAWL_DIR}"

cat > "${BIN_DIR}/lncrawl" <<'WRAPPER'
#!/usr/bin/env bash
exec "$HOME/.local/share/novelforge/tools/lncrawl-venv/bin/lncrawl" "$@"
WRAPPER
chmod 0755 "${BIN_DIR}/lncrawl"

LNCRAWL_PYTHON="${LNCRAWL_VENV}/bin/python" \
  bash "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/apply-lncrawl-patches.sh" "${LNCRAWL_DIR}"

# lncrawl-scraper 1.7.0 has two defects NovelForge relies on being fixed: it
# does not decode JSON-escaped `\/` in a JS redirect (NovelFire's readnovel.site
# hop becomes a 404), and `Pacer.learn()` re-applies a persisted widened
# interval on every request, so a throttled origin never recovers (NF-040).
# Patch the installed package and verify both behaviours; either step fails
# loudly.
SCRAPER_PATCH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/patch_lncrawl_scraper.py"
"${LNCRAWL_VENV}/bin/python" "${SCRAPER_PATCH}" || fail "Could not patch lncrawl-scraper"
"${LNCRAWL_VENV}/bin/python" "${SCRAPER_PATCH}" --verify || \
  fail "lncrawl-scraper still mishandles escaped redirects"
say "lncrawl installed: $("${BIN_DIR}/lncrawl" version 2>/dev/null | head -1 || echo 'installed')"
fi

if [ "${WANT_MANGA}" -eq 1 ]; then
# --- gallery-dl ------------------------------------------------------------
say "Creating Python virtualenv for gallery-dl"
if [ ! -x "${GALLERY_VENV}/bin/python" ]; then
  python3 -m venv "${GALLERY_VENV}"
fi
"${GALLERY_VENV}/bin/python" -m pip install --upgrade pip >/dev/null
"${GALLERY_VENV}/bin/python" -m pip install "gallery-dl==${GALLERY_DL_VERSION}"

cat > "${BIN_DIR}/gallery-dl" <<'WRAPPER'
#!/usr/bin/env bash
exec "$HOME/.local/share/novelforge/tools/gallery-dl-venv/bin/gallery-dl" "$@"
WRAPPER
chmod 0755 "${BIN_DIR}/gallery-dl"
say "gallery-dl installed: $("${BIN_DIR}/gallery-dl" --version 2>/dev/null | head -1 || echo 'installed')"
fi

if [ "${WANT_BOOKS}" -eq 1 ]; then
# --- aget ------------------------------------------------------------------
say "Fetching aget (Anna's Archive downloader)"
if [ -d "${AGET_DIR}/.git" ]; then
  git -C "${AGET_DIR}" pull --ff-only
else
  git clone --depth 1 "${AGET_REPO}" "${AGET_DIR}"
fi

# NF-028: NovelForge owns the book engine. The upstream helper takes the first
# result and opens a visible browser; this copy is LibGen-first, verifies
# title/author/ISBN and keeps every browser headless. Copy after the optional
# pull so an update cannot silently restore the old behaviour.
NOVELFORGE_SCRIPTS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cp "${NOVELFORGE_SCRIPTS}/aget.py" "${AGET_DIR}/scripts/aget.py"

if [ ! -x "${AGET_VENV}/bin/python" ]; then
  say "Creating Python virtualenv for aget"
  python3 -m venv "${AGET_VENV}"
fi

say "Installing aget Python dependencies"
"${AGET_VENV}/bin/python" -m pip install --upgrade pip >/dev/null
"${AGET_VENV}/bin/python" -m pip install \
  "click==${AGET_CLICK}" \
  "rich==${AGET_RICH}" \
  "playwright==${AGET_PLAYWRIGHT}" \
  "nodriver==${AGET_NODRIVER}"

say "Installing Playwright Chromium (aget drives a headless browser)"
"${AGET_VENV}/bin/python" -m playwright install chromium

cat > "${BIN_DIR}/aget" <<'WRAPPER'
#!/usr/bin/env bash
exec "$HOME/.local/share/novelforge/tools/aget-venv/bin/python" \
  "$HOME/.local/share/novelforge/tools/annas-archive-downloader/scripts/aget.py" "$@"
WRAPPER
chmod 0755 "${BIN_DIR}/aget"
say "aget installed: ${BIN_DIR}/aget"
fi

# --- report ----------------------------------------------------------------
case ":${PATH}:" in
  *":${BIN_DIR}:"*) ;;
  *)
    warn "${BIN_DIR} is not on PATH. Add it to your shell profile:"
    warn "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    ;;
esac

echo
say "Toolchain ready. NovelForge probes these commands on startup:"
TOOLS_TO_REPORT=()
[ "${WANT_WEBNOVEL}" -eq 1 ] && TOOLS_TO_REPORT+=(lncrawl)
[ "${WANT_MANGA}" -eq 1 ] && TOOLS_TO_REPORT+=(gallery-dl)
[ "${WANT_BOOKS}" -eq 1 ] && TOOLS_TO_REPORT+=(aget)
for tool in "${TOOLS_TO_REPORT[@]}"; do
  if command -v "${tool}" >/dev/null 2>&1; then
    printf '    %-12s %s\n' "${tool}" "$(command -v "${tool}")"
  else
    printf '    %-12s %s\n' "${tool}" "not found"
  fi
done
