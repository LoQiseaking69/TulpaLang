#!/bin/bash
# Cross-compilation script for Tulpa

set -e  # Exit immediately if a command exits with a non-zero status

echo "Starting cross-compilation for Tulpa..."

# List of target architectures to cross-compile for
targets=(
    "armv7-unknown-linux-gnueabihf"
    "aarch64-unknown-linux-gnu"
    "x86_64-unknown-linux-gnu"
    "i686-unknown-linux-gnu"
)

# Cross-compile for each target
for target in "${targets[@]}"; do
    echo "Cross-compiling for $target..."
    cargo build --release --target=$target
    echo "Cross-compilation for $target completed."
done

# Optional: Package the cross-compiled binaries
echo "Packaging cross-compiled binaries..."
mkdir -p target/release-cross-package

for target in "${targets[@]}"; do
    cp target/$target/release/tulpa target/release-cross-package/tulpa-$target
done

echo "Packaging completed."

# Final message
echo "Tulpa cross-compilation process completed successfully."
