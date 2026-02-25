#!/bin/bash
set -e

cd "$(dirname "$0")/.."

MAX_TIME="${FUZZ_MAX_TIME:-10}"
MAX_LEN="${FUZZ_MAX_LEN:-65536}"
RSS_LIMIT="${FUZZ_RSS_LIMIT:-1024}"

TARGETS=(
  fuzz_lcov
  fuzz_cobertura
  fuzz_clover
  fuzz_jacoco
  fuzz_istanbul
  fuzz_gocover
  fuzz_diff
)

echo "Running ${#TARGETS[@]} fuzz targets for ${MAX_TIME}s each..."

for target in "${TARGETS[@]}"; do
  echo "  Fuzzing $target..."
  cargo fuzz run --fuzz-dir tests/fuzz "$target" -- \
    -max_total_time="$MAX_TIME" \
    -max_len="$MAX_LEN" \
    -rss_limit_mb="$RSS_LIMIT"
done

echo "All fuzz tests completed!"
