#!/usr/bin/env bash
# Build a local revision with the binary package's exact package()/install hooks.
# The input archive must retain the release recipe's versioned directory layout.
set -euo pipefail
if [[ $# != 3 || ! $2 =~ ^[1-9][0-9]*$ ]]; then
    echo "Usage: $0 /path/to/release.tar.zst pkgrel /path/to/output" >&2
    exit 2
fi
archive=$(realpath "$1")
revision=$2
mkdir -p "$3"
output=$(realpath "$3")
recipe=$(cd -- "$(dirname -- "$0")/omarchyuu-bin" && pwd)
work=$(mktemp -d "$output/local-build.XXXXXX")
cp "$recipe/PKGBUILD" "$recipe/omarchyuu-bin.install" "$work/"
cp "$archive" "$work/"
name=$(basename "$archive")
checksum=$(sha256sum "$archive")
checksum=${checksum%% *}
{
    printf '\n# Local rebuild, pinned to the archive above.\npkgrel=%s\n' "$revision"
    printf 'source=(%q)\nsha256sums=(%q)\n' "$name" "$checksum"
} >> "$work/PKGBUILD"
cd "$work"
PKGDEST="$output" makepkg --cleanbuild --noconfirm
PKGDEST="$output" makepkg --printsrcinfo > .SRCINFO
echo "Package and reproducible local recipe: $output / $work"
