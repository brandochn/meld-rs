#!/bin/bash
set -e
echo "Building meld-rs..."
cargo build --release
echo "Build complete: target/release/meld-rs"
