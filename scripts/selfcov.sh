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

# ── Report ──────────────────────────────────────────────────────────────────

header "Reports in database"
$COVRS --db "$DB" reports

header "Summary: LCOV report"
$COVRS --db "$DB" summary --report selfcov-lcov

header "Summary: Cobertura report"
$COVRS --db "$DB" summary --report selfcov-cobertura

header "Per-file coverage (LCOV, sorted by coverage)"
$COVRS --db "$DB" files --report selfcov-lcov --sort-by-coverage

# ── Merge ───────────────────────────────────────────────────────────────────

header "Merging both reports"
$COVRS --db "$DB" merge selfcov-lcov --into selfcov-merged
$COVRS --db "$DB" merge selfcov-cobertura --into selfcov-merged
$COVRS --db "$DB" summary --report selfcov-merged

# ── Diff coverage ───────────────────────────────────────────────────────────

header "Diff coverage (HEAD vs HEAD~1)"
# This may fail if there's no git history, so don't bail
$COVRS --db "$DB" diff-coverage --report selfcov-lcov --git-diff "HEAD~1" || \
    echo "  (skipped — not enough git history or no changes)"

# ── Pick a source file and show uncovered lines ────────────────────────────

header "Uncovered lines sample"
# Grab the first covrs source file from the LCOV report
SAMPLE_FILE=$($COVRS --db "$DB" files --report selfcov-lcov \
    | grep '\.rs' \
    | head -1 \
    | awk '{print $1}')

if [ -n "$SAMPLE_FILE" ]; then
    step "Uncovered lines in: $SAMPLE_FILE"
    $COVRS --db "$DB" uncovered "$SAMPLE_FILE" --report selfcov-lcov
else
    echo "  (no source files found)"
fi

# ── Summary ─────────────────────────────────────────────────────────────────

header "Done!"
echo "Database: $DB ($(du -h "$DB" | cut -f1))"
echo "All covrs commands executed successfully."
