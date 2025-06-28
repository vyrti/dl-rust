#!/bin/bash

# This script builds the Rust application for multiple target platforms.
# Binaries will be placed in the build/ directory.

set -e # Exit immediately if a command exits with a non-zero status.

# The current version, extracted from Cargo.toml
VERSION=$(grep '^version =' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
REPO_NAME="dl"

echo "Starting build for $REPO_NAME version $VERSION..."

# Ensure the build directory exists and is clean
rm -rf build
mkdir -p build

# --- Build Targets ---
# Format: <target_triple> <output_binary_name>
targets=(
    "x86_64-unknown-linux-gnu dl.linux.x64"
    "aarch64-unknown-linux-gnu dl.linux.arm"
    "x86_64-pc-windows-gnu dl.win.x64.exe"
    "aarch64-pc-windows-msvc dl.win.arm.exe"
    "x86_64-apple-darwin dl.apple.intel"
    "aarch64-apple-darwin dl.apple.arm"
)

# Check for required cross-compilation tools
echo "Checking for necessary cross-compilation toolchains..."
rustup target list --installed

# Iterate over targets and build
for target_entry in "${targets[@]}"; do
    # Split the entry into target_triple and output_name
    read -r target_triple output_name <<< "$target_entry"

    echo "----------------------------------------------------"
    echo "Building for: $target_triple"
    echo "Output name:  $output_name"
    echo "----------------------------------------------------"

    # Check if the target is installed, if not, offer to install it
    if ! rustup target list --installed | grep -q "$target_triple"; then
        echo "Warning: Target '$target_triple' is not installed."
        read -p "Do you want to install it now with 'rustup target add $target_triple'? (y/N) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            rustup target add "$target_triple"
        else
            echo "Skipping build for $target_triple."
            continue
        fi
    fi

    # Build the application
    cargo build --release --target "$target_triple"

    # Move and rename the binary
    # The binary path depends on the target OS
    source_path="target/$target_triple/release/$REPO_NAME"
    if [[ "$target_triple" == *"-windows-"* ]]; then
        source_path+=".exe"
    fi

    if [ -f "$source_path" ]; then
        mv "$source_path" "build/$output_name"
        echo "Successfully built and moved to build/$output_name"
    else
        echo "Error: Build for $target_triple failed, binary not found at $source_path"
    fi
done

echo "===================================================="
echo "All builds completed."
echo "Binaries are located in the 'build/' directory."
ls -l build/
echo "===================================================="