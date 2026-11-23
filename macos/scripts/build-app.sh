#!/bin/sh
# Builds dist/Streamprobe.app: the SwiftUI app with streamprobe as a helper,
# ad-hoc signed for this Mac. Uses full Xcode without changing xcode-select.
set -eu
cd "$(dirname "$0")/../.."
DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
export DEVELOPER_DIR

cargo build --release --locked
swift build -c release --package-path macos
bin=$(swift build -c release --package-path macos --show-bin-path)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

app=dist/Streamprobe.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Helpers" "$app/Contents/Resources"
cp "$bin/Streamprobe" "$app/Contents/MacOS/Streamprobe"
cp target/release/streamprobe "$app/Contents/Helpers/streamprobe"
cp docs/FINDINGS.md "$app/Contents/Resources/FINDINGS.md"
cp macos/Resources/Info.plist "$app/Contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$version" "$app/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$version" "$app/Contents/Info.plist"

iconset=$(mktemp -d)/AppIcon.iconset
swift macos/scripts/make-icon.swift "$iconset"
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
rm -rf "$(dirname "$iconset")"

codesign --force --sign - "$app/Contents/Helpers/streamprobe"
codesign --force --sign - "$app"
echo "Built $app ($version)"
