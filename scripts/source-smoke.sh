#!/usr/bin/env bash
# Opt-in live smoke matrix for NovelForge webnovel sources.
#
# Requires a graphical session because the QA harness launches the Tauri app
# without a window. Each target downloads chapters 1-2 into a timestamped
# directory and writes one log per source.
#
# Usage:
#   NOVELFORGE_ALLOW_LIVE=1 scripts/source-smoke.sh [target ...]
#
# Targets: novelphoenix novelfull freewebnovel divinedaolibrary novelping novelfire
set -euo pipefail

if [[ "${NOVELFORGE_ALLOW_LIVE:-}" != "1" ]]; then
  echo "Refusing to run live network smoke. Set NOVELFORGE_ALLOW_LIVE=1." >&2
  exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT="${NOVELFORGE_SMOKE_DIR:-/tmp/opencode/nf015-smoke-$STAMP}"
mkdir -p "$OUT"

# name|url
MATRIX=(
  "novelphoenix|https://novelphoenix.net/mother-of-learning.html"
  "novelfull|https://novelfull.net/reverend-insanity.html"
  "freewebnovel|https://freewebnovel.com/novel/reverend-insanity"
  "divinedaolibrary|https://www.divinedaolibrary.com/story/alchemy-master/"
  "novelping|https://novelping.com/book/prison-of-eternal-sky"
  "novelfire|https://novelfire.net/book/reverend-insanity"
)

selected=("$@")
if [[ ${#selected[@]} -eq 0 ]]; then
  selected=("${MATRIX[@]%%|*}")
fi

is_selected() {
  local needle="$1" item
  for item in "${selected[@]}"; do
    [[ "$item" == "$needle" ]] && return 0
  done
  return 1
}

failures=0
for entry in "${MATRIX[@]}"; do
  name="${entry%%|*}"
  url="${entry#*|}"
  is_selected "$name" || continue

  log="$OUT/$name.log"
  echo "== $name"
  echo "   $url"
  echo "   log: $log"

  set +e
  (
    cd "$ROOT/src-tauri"
    NOVELFORGE_QA_QUERY="$url" \
    NOVELFORGE_QA_CHAPTERS="1-2" \
    NOVELFORGE_QA_DOWNLOAD_DIR="$OUT/$name" \
      cargo run --quiet
  ) >"$log" 2>&1
  status=$?
  set -e

  if [[ $status -ne 0 ]]; then
    echo "   FAIL exit=$status"
    grep -E "Chapter index|Chapter audit|Structured export|Success|transport report|failed|error" "$log" | tail -8 || true
    failures=$((failures + 1))
    continue
  fi

  summary="$(grep -E "Chapter audit|Structured export|Success|transport report" "$log" | tail -4)"
  if [[ -z "$summary" ]]; then
    echo "   FAIL: no audit/export lines"
    failures=$((failures + 1))
  else
    echo "$summary" | sed 's/^/   /'
  fi
done

echo
if [[ $failures -gt 0 ]]; then
  echo "$failures target(s) failed; logs in $OUT" >&2
  exit 1
fi
echo "All selected targets passed. Logs in $OUT"
