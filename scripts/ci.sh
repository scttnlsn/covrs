#!/usr/bin/env bash
set -euo pipefail

# CI check script — runs all quality gates locally.
# Usage: ./scripts/ci.sh

cd "$(dirname "$0")/.."

passed=0
failed=0

run() {
  echo "──────────────────────────────────────"
  echo "▶ $*"
  echo "──────────────────────────────────────"
  if "$@"; then
    passed=$((passed + 1))
  else
    failed=$((failed + 1))
    echo "✗ FAILED: $*"
  fi
  echo
}

run cargo fmt -- --check
run cargo check --all-targets
run cargo clippy --all-targets -- -D warnings
run cargo test
run cargo doc --no-deps --document-private-items

echo "══════════════════════════════════════"
if [ "$failed" -eq 0 ]; then
  echo "✅ All $passed checks passed!"
else
  echo "❌ $failed check(s) failed ($passed passed)"
  exit 1
fi
