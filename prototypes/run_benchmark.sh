#!/bin/bash
# Holodex Prototype Benchmark Runner
#
# Usage:
#   ./run_benchmark.sh <input.ndjson>
#   ./run_benchmark.sh ~/dev/tmp/data-dumpt/next-export.ndjson
#
# Prerequisites:
#   - Rust toolchain (cargo)
#   - Input NDJSON file with documents
#
# What it measures:
#   1. Index build time
#   2. Index size (bytes per doc)
#   3. FPR for sample queries
#   4. Query throughput

set -e

INPUT_FILE="${1:-input.ndjson}"

if [ ! -f "$INPUT_FILE" ]; then
    echo "Error: Input file not found: $INPUT_FILE"
    echo "Usage: ./run_benchmark.sh <input.ndjson>"
    exit 1
fi

echo "=== Holodex Prototype Benchmark ==="
echo "Input: $INPUT_FILE"
echo "Documents: $(wc -l < "$INPUT_FILE")"
echo ""

# Build the prototype
echo "Building prototype..."
cargo build --release --bin holodex_bench 2>&1 | tail -5

# Run benchmark
echo ""
echo "Running benchmark..."
cargo run --release --bin holodex_bench -- "$INPUT_FILE"

echo ""
echo "=== Benchmark Complete ==="
