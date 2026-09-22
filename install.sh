#!/bin/sh
# Installs the latest knav release for this machine.
#
#   curl -fsSL https://raw.githubusercontent.com/guilhermemcandido/knav/main/install.sh | sh
#
# KNAV_VERSION=v0.1.0 pins a version instead of the latest.
# KNAV_INSTALL_DIR changes where the binary goes (default: $HOME/.local/bin).
set -eu

repo="guilhermemcandido/knav"
version="${KNAV_VERSION:-latest}"
install_dir="${KNAV_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$1"; }
die() { say "error: $1" >&2; exit 1; }

os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Darwin) os=apple-darwin ;;
    Linux) os=unknown-linux-gnu ;;
    *) die "unsupported OS: $os (build from source instead: cargo install --git https://github.com/$repo)" ;;
esac
case "$arch" in
    arm64|aarch64) arch=aarch64 ;;
    x86_64|amd64) arch=x86_64 ;;
    *) die "unsupported architecture: $arch" ;;
esac
target="$arch-$os"
archive="knav-$target.tar.gz"

if [ "$version" = "latest" ]; then
    base="https://github.com/$repo/releases/latest/download"
else
    base="https://github.com/$repo/releases/download/$version"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"

say "Downloading $archive ($version)..."
curl -fsSL "$base/$archive" -o "$archive" || die "couldn't download $base/$archive (does that release have a build for $target?)"
curl -fsSL "$base/$archive.sha256" -o "$archive.sha256" || die "couldn't download the checksum file"

say "Verifying checksum..."
if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$archive.sha256" || die "checksum mismatch, not installing"
else
    expected=$(awk '{print $1}' "$archive.sha256")
    actual=$(shasum -a 256 "$archive" | awk '{print $1}')
    [ "$expected" = "$actual" ] || die "checksum mismatch, not installing"
fi

tar xzf "$archive"
mkdir -p "$install_dir"
mv "knav-$target/knav" "$install_dir/knav"
chmod +x "$install_dir/knav"

say "Installed to $install_dir/knav"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) say "Add it to your PATH: export PATH=\"$install_dir:\$PATH\"" ;;
esac
knav_version=$("$install_dir/knav" --version 2>/dev/null || true)
[ -n "$knav_version" ] && say "$knav_version"
