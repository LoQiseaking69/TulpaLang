#!/bin/bash
# Build script for Tulpa

set -e  # Exit immediately if a command exits with a non-zero status

echo "Starting Tulpa build process..."

# Step 1: Clean previous builds
echo "Cleaning previous builds..."
cargo clean
echo "Clean completed."

# Step 2: Build the project in release mode
echo "Building Tulpa project in release mode..."
cargo build --release
echo "Build completed."

# Step 3: Run tests
echo "Running tests..."
cargo test --release
echo "Tests completed."

# Step 4: Generate documentation
echo "Generating documentation..."
cargo doc --release --no-deps
echo "Documentation generated."

# Step 5: Optionally, package the build
echo "Packaging the build..."
# Create a directory for the release package
mkdir -p target/release-package
# Copy the binary and documentation to the release package directory
cp target/release/tulpa target/release-package/
cp -r target/doc target/release-package/
echo "Packaging completed."

# Final message
echo "Tulpa build process completed successfully."
