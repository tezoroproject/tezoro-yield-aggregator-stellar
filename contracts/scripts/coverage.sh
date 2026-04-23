#!/usr/bin/env bash
# Branch-coverage runner for Tezoro Soroban contracts.
#
# Branch coverage (`-Z coverage-options=branch`) is still a nightly-only
# Rust feature. We pin to a specific nightly because newer nightlies reject
# upstream UB in `ethnum` (a transitive Soroban SDK dep).
#
# Requirements (installed once):
#   rustup toolchain install nightly-2026-01-15 --component llvm-tools-preview
#   cargo install cargo-llvm-cov --locked
#
# Usage:
#   ./scripts/coverage.sh            -> text summary to stdout
#   ./scripts/coverage.sh html       -> HTML report at ./target/coverage/html
#   ./scripts/coverage.sh lcov       -> LCOV file at ./target/coverage/lcov.info

set -euo pipefail

TOOLCHAIN=nightly-2026-01-15
# Production contracts only; `mock-strategy` is test infrastructure.
PACKAGES=(-p tezoro-vault -p blend-strategy -p tezoro-common)
MODE=${1:-summary}

# Tests live across several crates (including mock-strategy's helper contract
# used in integration tests), so we must run `--tests` across the full set
# while measuring coverage only for the production packages above.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

OUTPUT_DIR="target/coverage"
mkdir -p "$OUTPUT_DIR"

case "$MODE" in
  summary)
    cargo "+$TOOLCHAIN" llvm-cov --branch --summary-only \
      "${PACKAGES[@]}" --tests
    ;;
  html)
    cargo "+$TOOLCHAIN" llvm-cov --branch --html \
      --output-dir "$OUTPUT_DIR/html" \
      "${PACKAGES[@]}" --tests
    echo "HTML report: $OUTPUT_DIR/html/index.html"
    ;;
  lcov)
    cargo "+$TOOLCHAIN" llvm-cov --branch --lcov \
      --output-path "$OUTPUT_DIR/lcov.info" \
      "${PACKAGES[@]}" --tests
    echo "LCOV file: $OUTPUT_DIR/lcov.info"
    ;;
  json)
    cargo "+$TOOLCHAIN" llvm-cov --branch --json \
      --output-path "$OUTPUT_DIR/coverage.json" \
      "${PACKAGES[@]}" --tests
    echo "JSON report: $OUTPUT_DIR/coverage.json"
    ;;
  *)
    echo "Unknown mode: $MODE" >&2
    echo "Usage: $0 [summary|html|lcov|json]" >&2
    exit 2
    ;;
esac
