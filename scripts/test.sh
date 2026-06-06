#!/bin/bash
set -e
echo "Running tests..."
cargo test --no-default-features
echo "All tests passed"
