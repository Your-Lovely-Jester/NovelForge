#!/usr/bin/env bash
# Offline gate: run this before shipping any change.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== Rust formatting"
cargo fmt --manifest-path src-tauri/Cargo.toml --check

echo "== Personal data and fixture-embed scan (NF-060)"
./scripts/check-personal-data.sh

echo "== Rust lints (clippy, warnings are errors)"
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

echo "== Rust tests (unit + offline integration)"
cargo test --manifest-path src-tauri/Cargo.toml

echo "== Frontend component tests"
npm test

echo "== Book source scoring tests (offline, NF-031)"
AGET_PYTHON="${NOVELFORGE_AGET_PYTHON:-$HOME/.local/share/novelforge/tools/aget-venv/bin/python}"
if [ ! -x "$AGET_PYTHON" ]; then
  AGET_PYTHON="$(command -v python3)"
fi
"$AGET_PYTHON" -m unittest discover -s scripts -t scripts -p 'test_*.py'

echo "== Frontend dead code and unused dependencies (knip)"
npm run knip

echo "== Frontend typecheck + production build"
npm run build

echo "All offline checks passed."
