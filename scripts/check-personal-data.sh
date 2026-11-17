#!/usr/bin/env bash
# NF-060: a release tree must not carry personal absolute paths, and the
# installer must never embed the saved-page test fixtures.
#
# The fixtures stay in the private development tree because the offline tests
# depend on real saved pages, but they are not product code and must not reach
# a binary. Run this from scripts/check.sh and again before packaging.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

status=0

# 1. Personal home directories in product code and build inputs. Test files are
#    excluded: they are not shipped and may legitimately use /home/test/...
#    The trailing slash keeps import specifiers such as @/components/home/Hero
#    from matching.
hits="$(
  grep -rInE '/(home|Users)/[A-Za-z0-9._-]+/' \
    src src-tauri/src scripts/*.sh scripts/*.py scripts/*.ps1 \
    package.json src-tauri/tauri.conf.json \
    --exclude='*.test.ts' --exclude='*.test.tsx' --exclude='*.spec.ts' --exclude='*.spec.tsx' \
    --exclude='check-personal-data.sh' \
    2>/dev/null || true
)"
if [ -n "$hits" ]; then
  echo "Personal absolute paths found in product code:" >&2
  printf '%s\n' "$hits" >&2
  status=1
fi

# 2. Fixtures must never be declared as a bundle resource.
if grep -q '"resources"' src-tauri/tauri.conf.json; then
  echo "tauri.conf.json declares bundle resources; fixtures could ship" >&2
  status=1
fi

# 3. Release code may only embed files from resources/. Paths after the first
#    #[cfg(test)] marker are test-only and never compiled into the binary.
embedded=""
while IFS= read -r file; do
  [ -z "$file" ] && continue
  test_line="$(grep -n '#\[cfg(test)\]' "$file" | head -n1 | cut -d: -f1 || true)"
  [ -z "$test_line" ] && test_line=999999
  while IFS= read -r hit; do
    [ -z "$hit" ] && continue
    line="${hit%%:*}"
    if [ "$line" -lt "$test_line" ] && ! printf '%s' "$hit" | grep -q 'resources/'; then
      embedded="${embedded}${file}:${hit}"$'\n'
    fi
  done < <(grep -n 'include_bytes!\|include_str!' "$file" || true)
done < <(grep -rl 'include_bytes!\|include_str!' src-tauri/src || true)
if [ -n "$embedded" ]; then
  echo "Release code embeds a file outside resources/:" >&2
  printf '%s' "$embedded" >&2
  status=1
fi

# 4. Known copyrighted fixture markers must not appear in product code.
for marker in \
  'Reverend Insanity: Chapter 7 | NovelPhoenix' \
  'fourth-wing.html'
do
  if found="$(grep -rIlF -- "$marker" src src-tauri/src 2>/dev/null || true)"; then
    if [ -n "$found" ]; then
      echo "Fixture marker \"$marker\" found in product code:" >&2
      printf '%s\n' "$found" >&2
      status=1
    fi
  fi
done

# 5. If a release binary is present, prove fixture prose is not inside it.
for bin in src-tauri/target/release/novelforge src-tauri/target/release/novelforge.exe; do
  if [ -f "$bin" ] && grep -aqF 'Reverend Insanity: Chapter 7 | NovelPhoenix' "$bin"; then
    echo "Fixture prose is embedded in $bin" >&2
    status=1
  fi
done

if [ "$status" -ne 0 ]; then
  exit 1
fi
echo "No personal paths or fixture embeds found."
