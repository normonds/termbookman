#!/bin/bash
# Build the production version of the dashboard
cargo build --release

# Copy the resulting binary to the project root
cp target/release/rust-dashboard ./termbookman

echo "------------------------------------------------"
echo "Build complete! Production binary: ./termbookman"
echo "To run: ./termbookman"
echo "------------------------------------------------"
