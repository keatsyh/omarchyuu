#!/usr/bin/env bash
# Build a Pages-ready Pacman repository from a verified release archive.
# Run as an unprivileged Arch user; no application compilation is required.
set -euo pipefail

archive=$(realpath "${1:?usage: build-repository.sh <archive.tar.zst> <output-directory>}")
output=${2:?usage: build-repository.sh <archive.tar.zst> <output-directory>}
filename=$(basename "$archive")
if [[ ! $filename =~ ^omarchyuu-([0-9]+\.[0-9]+\.[0-9]+)-x86_64-linux\.tar\.zst$ ]]; then
    echo "Expected a stable x86_64 release archive, got: $filename" >&2
    exit 1
fi
version=${BASH_REMATCH[1]}
project=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/package" "$work/repository/x86_64" "$output"
output=$(realpath "$output")

cp "$project/packaging/aur/omarchyuu-bin/"{PKGBUILD,omarchyuu-bin.install} "$work/package/"
checksum=$(sha256sum "$archive" | cut -d ' ' -f 1)
# Reuse the maintained package layout, pinning this archive's version/hash.
awk -v version="$version" -v checksum="$checksum" '
    /^pkgver=/ { print "pkgver=" version; next }
    /^sha256sums=/ { print "sha256sums=(\047" checksum "\047)"; next }
    { print }
' "$work/package/PKGBUILD" > "$work/package/PKGBUILD.new"
mv "$work/package/PKGBUILD.new" "$work/package/PKGBUILD"
cp "$archive" "$work/package/$filename"
(
    cd "$work/package"
    # Only repackage prebuilt files: runtime libraries are needed on the
    # user's system, not on this packaging host. Preserve them in .PKGINFO.
    PKGDEST="$work/repository/x86_64" makepkg --cleanbuild --noconfirm --nodeps
)
(
    cd "$work/repository/x86_64"
    repo-add omarchyuu.db.tar.gz ./*.pkg.tar.zst
    # Pages artifacts cannot contain symlinks; serve real database aliases.
    for database in omarchyuu.db omarchyuu.files; do
        rm "$database"
        cp "$database.tar.gz" "$database"
    done
    sha256sum ./*.pkg.tar.zst > SHA256SUMS
)
cat > "$work/repository/index.html" <<'HTML'
<!doctype html>
<meta charset="utf-8">
<title>omarchyuu Pacman repository</title>
<h1>omarchyuu</h1>
<p>Pacman repository for Omarchy / Arch Linux (x86_64).</p>
<p><a href="https://github.com/keatsyh/omarchyuu#安装">Installation instructions</a></p>
HTML
touch "$work/repository/.nojekyll"
cp -a "$work/repository/." "$output/"
printf 'Pacman repository ready: %s (omarchyuu-bin %s)\n' "$output" "$version"
