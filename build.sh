#!/bin/bash
# Build the production version of the dashboard
cargo build --release

# Remove old binary to avoid 'Text file busy' and copy the new one
rm -f ./termbookman
cp target/release/rust-dashboard ./termbookman || { echo "ERROR: Could not replace ./termbookman. Is it still running?"; exit 1; }

echo "------------------------------------------------"
echo "Build complete! Production binary: ./termbookman"
echo "To run: ./termbookman"
echo "------------------------------------------------"
