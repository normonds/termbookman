#!/bin/bash
# Build the production version of the dashboard for ARM64 (Ubuntu ARM)
# Requires aarch64-unknown-linux-gnu target and cross-compiler to be installed

export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc

echo "Starting ARM64 release build..."
cargo build --release --target aarch64-unknown-linux-gnu

# Copy the resulting binary to the project root
if [ -f target/aarch64-unknown-linux-gnu/release/rust-dashboard ]; then
    cp target/aarch64-unknown-linux-gnu/release/rust-dashboard ./tbm.arm
    echo "------------------------------------------------"
    echo "Build complete! ARM binary: ./termbookman.arm"
    echo "------------------------------------------------"
else
    echo "Build failed. Check error messages above."
    exit 1
fi
