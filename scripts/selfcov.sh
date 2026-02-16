#!/usr/bin/env bash
#
# selfcov.sh — Generate coverage for the covrs project itself, then ingest
# and report on it using covrs. A full end-to-end smoke test.
#
# Prerequisites:
#   cargo install cargo-llvm-cov
#   rustup component add llvm-tools
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

# Workspace for generated artifacts
OUT_DIR="target/selfcov"
mkdir -p "$OUT_DIR"

DB="$OUT_DIR/covrs-selfcov.db"
LCOV_FILE="$OUT_DIR/coverage.lcov"
COBERTURA_FILE="$OUT_DIR/coverage.xml"
COVRS="./target/release/covrs"

# ── Helpers ─────────────────────────────────────────────────────────────────

header() {
    echo ""
    echo "═══════════════════════════════════════════════════════════════"
    echo "  $1"
    echo "═══════════════════════════════════════════════════════════════"
}

step() {
    echo ""
    echo "── $1"
}

# ── Build covrs ─────────────────────────────────────────────────────────────

header "Building covrs (release)"
cargo build --release --quiet

# ── Generate coverage ───────────────────────────────────────────────────────

header "Running tests with coverage instrumentation"

step "Generating LCOV report"
cargo llvm-cov test --lcov --output-path "$LCOV_FILE" 2>&1 | tail -5
echo "  → $LCOV_FILE ($(wc -c < "$LCOV_FILE" | tr -d ' ') bytes)"

step "Generating Cobertura XML report"
cargo llvm-cov test --cobertura --output-path "$COBERTURA_FILE" 2>&1 | tail -5
echo "  → $COBERTURA_FILE ($(wc -c < "$COBERTURA_FILE" | tr -d ' ') bytes)"

# ── Ingest into covrs ──────────────────────────────────────────────────────

header "Ingesting coverage into covrs"

# Start fresh
rm -f "$DB"

step "Ingesting LCOV report"
$COVRS --db "$DB" ingest "$LCOV_FILE" --name "selfcov-lcov" --overwrite

step "Ingesting Cobertura report"
$COVRS --db "$DB" ingest "$COBERTURA_FILE" --name "selfcov-cobertura" --overwrite

# ── Show uncovered lines for all files ──────────────────────────────────────

header "Line by line coverage"
FILES=$($COVRS --db "$DB" files \
    | grep '\.rs' \
    | awk '{print $1}')

if [ -n "$FILES" ]; then
    while IFS= read -r FILE; do
        OUTPUT=$($COVRS --db "$DB" lines "$FILE" 2>&1)
        if [ -n "$OUTPUT" ]; then
            step "Uncovered lines in: $FILE"
            echo "$OUTPUT"
        fi
    done <<< "$FILES"
else
    echo "  (no source files found)"
fi

# ── Report ──────────────────────────────────────────────────────────────────

header "Reports"
$COVRS --db "$DB" reports

header "Summary"
$COVRS --db "$DB" summary

header "Per-file coverage"
$COVRS --db "$DB" files  --sort-by-coverage

# ── Diff coverage ───────────────────────────────────────────────────────────

header "Diff coverage (text)"
# This may fail if there's no git history, so don't bail
$COVRS --db "$DB" diff-coverage --git-diff "HEAD~1" --path-prefix "$PROJECT_DIR" || \
    echo "  (skipped — not enough git history or no changes)"
header "Diff coverage (markdown)"
$COVRS --db "$DB" diff-coverage --git-diff "HEAD~1" --path-prefix "$PROJECT_DIR" --style markdown || \
    echo "  (skipped — not enough git history or no changes)"

# ── Summary ─────────────────────────────────────────────────────────────────

header "Done!"
echo "Database: $DB ($(du -h "$DB" | cut -f1))"
echo "All covrs commands executed successfully."
