#!/usr/bin/env bash
# Linux packages from target/release/rustql: a tarball, an AppImage and a
# .deb, in OUTDIR. The release workflow runs it inside the manylinux_2_28
# container; it also works on any Linux machine with the tools below.
#
#   scripts/package-linux.sh VERSION OUTDIR
#
# Needs: cargo-deb, objdump, and in $APPIMAGETOOL / $APPIMAGE_RUNTIME the
# appimagetool AppImage and its type2 runtime.
set -euo pipefail

version=$1
out=$(realpath -m "$2")
root=$(cd "$(dirname "$0")/.." && pwd)
bin="$root/target/release/rustql"
id=com.rustql.RustQL
max_glibc=2.28

cd "$root"
mkdir -p "$out"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The whole point of building in an old container: the binary must not need
# a newer glibc than the oldest distribution it promises to run on
needed=$(objdump -T "$bin" | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)*' | sed 's/GLIBC_//' | sort -uV | tail -1)
if [ "$(printf '%s\n%s\n' "$needed" "$max_glibc" | sort -V | tail -1)" != "$max_glibc" ]; then
    echo "error: the binary needs glibc $needed, more than $max_glibc" >&2
    exit 1
fi
echo "glibc needed: $needed (at most $max_glibc)"

# Tarball: the binary with its desktop entry, metadata and icon
name="rustql-$version-linux-x86_64"
mkdir "$work/$name"
cp "$bin" "$work/$name/"
cp "packaging/linux/$id.desktop" "packaging/linux/$id.metainfo.xml" LICENSE README.md "$work/$name/"
cp assets/icon/icon.png "$work/$name/$id.png"
tar -C "$work" -czf "$out/$name.tar.gz" "$name"

# AppImage: an AppDir with the usual layout; AppRun is the binary itself
appdir="$work/RustQL.AppDir"
install -Dm755 "$bin" "$appdir/usr/bin/rustql"
install -Dm644 "packaging/linux/$id.desktop" "$appdir/usr/share/applications/$id.desktop"
install -Dm644 "packaging/linux/$id.metainfo.xml" "$appdir/usr/share/metainfo/$id.metainfo.xml"
install -Dm644 assets/icon/icon.png "$appdir/usr/share/icons/hicolor/256x256/apps/$id.png"
cp "packaging/linux/$id.desktop" "$appdir/"
cp assets/icon/icon.png "$appdir/$id.png"
ln -s "$id.png" "$appdir/.DirIcon"
ln -s usr/bin/rustql "$appdir/AppRun"
# Without FUSE (a container), appimagetool runs from its extracted contents
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$APPIMAGETOOL" --no-appstream \
    --runtime-file "$APPIMAGE_RUNTIME" "$appdir" "$out/rustql-$version-x86_64.AppImage"

# .deb (metadata in Cargo.toml, [package.metadata.deb])
cargo deb --no-build --output "$out/"

ls -l "$out"
