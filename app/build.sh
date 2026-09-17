#!/bin/bash
# Build mac-headroom.app: the SwiftUI menu bar app with the Rust CLI inside it.
# Signed with a Developer ID when one is installed. Distribution also needs the
# hardened runtime and notarisation, which this does not do yet.
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

# Sign with a real identity when there is one. macOS ties privacy permissions
# to the signature; an ad hoc signature changes with every build, so each
# rebuild would look like a new app and every permission would be forgotten.
IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | grep -o '"Developer ID Application: [^"]*"' | head -1 | tr -d '"')"
if [ -n "$IDENTITY" ]; then
  echo "Signing as $IDENTITY"
  SIGN="$IDENTITY"
else
  echo "No Developer ID found; signing ad hoc, so permissions reset on every build."
  SIGN="-"
fi
codesign --force --sign "$SIGN" "$APP/Contents/MacOS/mac-headroom"
codesign --force --sign "$SIGN" "$APP"
echo "Built $(pwd)/$APP"
