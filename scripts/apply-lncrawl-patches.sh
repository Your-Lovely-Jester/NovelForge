#!/usr/bin/env bash
# Apply NovelForge's source patches to a lightnovel-crawler checkout.
#
# The NovelFull adapter shipped by lncrawl v4.14.0 still expects
# `#rating[data-novel-id]` and a pre-rendered `ul.list-chapter`. novelfull.net
# now renders page 1 and serves the rest through `/ajax-chapter-list`, so the
# upstream adapter raises "Failed to parse chapter list". Its template also
# reads `h3.title` and `.info` author links, while the live page uses `h1.tit`
# and `.m-info .item`, so the title picked up a `| NovelFull` suffix and the
# author was lost.
#
# The Divine Dao Library adapter still targets the pre-Fictioneer theme, so it
# sees no title and no chapters. The patch follows `article.story__article`,
# `h1.story__identity-title` and the `chapter-group__list-item-link` TOC.
#
# lncrawl has no NovelPing adapter at all. The patch adds one that reads the
# `/ajax/chapter-archive` template, because the story page only renders the
# first 30 of the novel's chapters.
#
# NovelPhoenix moved to `novelphoenix.net` and paginates its full TOC; the
# upstream adapter still points at `.com`, follows a stale `/chapters` page and
# has no search. The patch re-anchors the adapter and its selectors.
#
# The published source index describes upstream capabilities, so a patched
# adapter that gained search still looks search-less. The patch takes those flags
# from the locally loaded module.
#
# NovelForge drives chapter selection through `--chapters`, which upstream
# v4.14.0 does not implement. That patch adds the option and filters chapter
# serials in the chapter service.
#
# Finally, the GitHub source-index sync costs ~90 s per crawl and can fail the
# loader thread with an `Exhausted` traceback when GitHub is slow. NovelForge
# pins the crawler and ships these patches, so the patch makes the sync opt-in
# through `LNCRAWL_SYNC_REMOTE_INDEX=1` instead of running on every command.
#
# Usage:
#   ./scripts/apply-lncrawl-patches.sh [checkout]
#
# The checkout defaults to ~/.local/share/novelforge/tools/lightnovel-crawler.
# Re-running is safe: already-applied patches are skipped.
set -euo pipefail

CHECKOUT="${1:-${HOME}/.local/share/novelforge/tools/lightnovel-crawler}"
PATCH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/patches"

say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[!]\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31m[x]\033[0m %s\n' "$*" >&2; exit 1; }

[ -d "${CHECKOUT}/.git" ] || fail "Not a lightnovel-crawler checkout: ${CHECKOUT}"
command -v git >/dev/null 2>&1 || fail "Missing required command: git"

# A Windows clone with core.autocrlf=true checks the patch files out with CRLF,
# and git then rejects every hunk against the LF checkout. Normalise to LF.
NORMALISED_DIR="$(mktemp -d)"
trap 'rm -rf "${NORMALISED_DIR}"' EXIT

apply_patch() {
  local file="$1" source patch
  source="${PATCH_DIR}/${file}"
  [ -f "${source}" ] || fail "Patch not found: ${source}"
  patch="${NORMALISED_DIR}/${file}"
  sed 's/\r$//' "${source}" >"${patch}"

  if git -C "${CHECKOUT}" apply --reverse --check "${patch}" >/dev/null 2>&1; then
    say "Already applied: ${file}"
    return
  fi

  if ! git -C "${CHECKOUT}" apply --check "${patch}" >/dev/null 2>&1; then
    fail "Patch does not apply cleanly: ${file} (check the lncrawl version)"
  fi

  git -C "${CHECKOUT}" apply "${patch}"
  say "Applied: ${file}"
}

say "Patching lncrawl at ${CHECKOUT}"
apply_patch "lncrawl-v4.14.0-novelfull-ajax.patch"
apply_patch "lncrawl-v4.14.0-novelfull-metadata.patch"
apply_patch "lncrawl-v4.14.0-divinedaolibrary-redesign.patch"
apply_patch "lncrawl-v4.14.0-novelping-adapter.patch"
apply_patch "lncrawl-v4.14.0-novelphoenix-redesign.patch"
apply_patch "lncrawl-v4.14.0-source-capabilities.patch"
apply_patch "lncrawl-v4.14.0-chapter-selection.patch"
apply_patch "lncrawl-v4.14.0-offline-source-index.patch"

# A source checkout is imported as Python modules; compile the touched files so
# a syntax error fails the installer instead of the first crawl.
PYTHON="${LNCRAWL_PYTHON:-python3}"
"${PYTHON}" -m py_compile \
  "${CHECKOUT}/sources/en/n/novelfull.py" \
  "${CHECKOUT}/sources/en/d/divinedaolibrary.py" \
  "${CHECKOUT}/sources/en/n/novelping.py" \
  "${CHECKOUT}/sources/en/n/novelphoenix.py" \
  "${CHECKOUT}/lncrawl/services/sources/service.py" \
  "${CHECKOUT}/lncrawl/commands/crawl/crawl.py" \
  "${CHECKOUT}/lncrawl/services/chapters.py" \
  "${CHECKOUT}/lncrawl/context.py" \
  || fail "A patched file does not compile under ${CHECKOUT}"
say "lncrawl source patches ready"
