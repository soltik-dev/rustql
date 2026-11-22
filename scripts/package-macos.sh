#!/usr/bin/env bash
# RustQL.app (universal: Apple Silicon and Intel) in a .dmg, in OUTDIR, from
# the two release binaries. Runs on macOS (lipo, codesign, hdiutil).
#
#   scripts/package-macos.sh VERSION ARM64_BINARY X86_64_BINARY OUTDIR
set -euo pipefail

version=$1
arm=$(realpath "$2")
x86=$(realpath "$3")
mkdir -p "$4"
out=$(realpath "$4")
root=$(cd "$(dirname "$0")/.." && pwd)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
app="$work/dmg/RustQL.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

lipo -create -output "$app/Contents/MacOS/rustql" "$arm" "$x86"
lipo -info "$app/Contents/MacOS/rustql"
sed "s/@VERSION@/$version/g" "$root/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
cp "$root/assets/icon/icon.icns" "$app/Contents/Resources/RustQL.icns"

# Ad-hoc signature: Apple Silicon does not run unsigned code at all. Without a
# Developer ID it is not notarized, so the first launch needs "Open Anyway"
# in System Settings → Privacy & Security.
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

# The usual disk image: the app next to a link to /Applications
ln -s /Applications "$work/dmg/Applications"
hdiutil create -volname "RustQL $version" -srcfolder "$work/dmg" -ov -format UDZO \
    "$out/rustql-$version-macos-universal.dmg"

ls -l "$out"
