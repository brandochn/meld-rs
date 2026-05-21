#!/bin/bash
set -e
echo "Running tests..."
cargo test -- --test-threads=1
echo "All tests passed"
