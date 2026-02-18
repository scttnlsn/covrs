#!/bin/bash
set -e

cd "$(dirname "$0")/.."

echo "Running fuzz tests for 10 seconds each..."

cargo fuzz run --fuzz-dir tests/fuzz fuzz_lcov -- -max_total_time=10
cargo fuzz run --fuzz-dir tests/fuzz fuzz_cobertura -- -max_total_time=10
cargo fuzz run --fuzz-dir tests/fuzz fuzz_jacoco -- -max_total_time=10
cargo fuzz run --fuzz-dir tests/fuzz fuzz_diff -- -max_total_time=10

echo "All fuzz tests completed!"
