#!/bin/bash
# Build mac-headroom.app: the SwiftUI menu bar app with the Rust CLI inside it.
# Signed ad hoc, which runs on this Mac. Distribution needs Developer ID signing
# and notarisation, which this does not do yet.
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(cd .. && pwd)"

echo "Building the command line tool…"
cargo build --release --manifest-path "$ROOT/Cargo.toml"

echo "Building the app…"
swift build -c release

APP="build/mac-headroom.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp .build/release/MacHeadroom "$APP/Contents/MacOS/"
cp "$ROOT/target/release/mac-headroom" "$APP/Contents/MacOS/"
cp Info.plist "$APP/Contents/"

codesign --force --sign - "$APP/Contents/MacOS/mac-headroom"
codesign --force --sign - "$APP"
echo "Built $(pwd)/$APP"
