#!/usr/bin/env bash
# Source tarball with every dependency vendored, in OUTDIR: it builds offline
# (`cargo build --release --offline`), as COPR and Flathub require.
#
#   scripts/package-source.sh VERSION OUTDIR
set -euo pipefail

version=$1
mkdir -p "$2"
out=$(realpath "$2")
root=$(cd "$(dirname "$0")/.." && pwd)
name="rustql-$version"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git -C "$root" archive --format=tar --prefix="$name/" HEAD | tar -x -C "$work"

cd "$work/$name"
mkdir .cargo
cargo vendor --locked --versioned-dirs vendor > .cargo/config.toml
cd "$work"
tar -cJf "$out/$name-vendored.tar.xz" "$name"

ls -l "$out"
