#!/bin/sh
# Installs the latest knav release for this machine.
#
#   curl -fsSL https://raw.githubusercontent.com/guilhermemcandido/knav/main/install.sh | sh
#
# Pin a version with `sh -s -- v0.1.0` or KNAV_VERSION=v0.1.0.
# KNAV_INSTALL_DIR changes where the binary goes (default: $HOME/.local/bin).
set -eu

repo="guilhermemcandido/knav"
version="${1:-${KNAV_VERSION:-latest}}"
install_dir="${KNAV_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$1"; }
die() { say "error: $1" >&2; exit 1; }

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q "$1" -O "$2"; }
else
    die "needs curl or wget"
fi

# No prebuilt binary for this machine or version: build it with cargo if we can.
from_source() {
    say "$1"
    command -v cargo >/dev/null 2>&1 || die "install Rust (https://rustup.rs) and run this again, or: cargo install --git https://github.com/$repo"
    say "Building from source with cargo, this takes a few minutes..."
    if [ "$version" = "latest" ]; then
        cargo install --locked --git "https://github.com/$repo" --root "$work/cargo" knav
    else
        cargo install --locked --git "https://github.com/$repo" --tag "$version" --root "$work/cargo" knav
    fi
    mkdir -p "$install_dir"
    mv "$work/cargo/bin/knav" "$install_dir/knav"
    finish
}

finish() {
    say "Installed to $install_dir/knav"
    case ":$PATH:" in
        *":$install_dir:"*) ;;
        *)
            case "${SHELL:-}" in
                */zsh) rc="$HOME/.zshrc" ;;
                */bash) rc="$HOME/.bashrc" ;;
                */fish) rc="$HOME/.config/fish/config.fish" ;;
                *) rc="$HOME/.profile" ;;
            esac
            say "$install_dir isn't on your PATH. Add this to $rc:"
            say "  export PATH=\"$install_dir:\$PATH\""
            ;;
    esac
    "$install_dir/knav" --version 2>/dev/null || true
    exit 0
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"

os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Darwin)
        os=apple-darwin
        # An Intel shell under Rosetta still reports x86_64 on Apple silicon.
        [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = 1 ] && arch=arm64
        ;;
    # Static builds: they run on any distribution, Alpine included.
    Linux) os=unknown-linux-musl ;;
    *) from_source "No prebuilt binary for $os." ;;
esac
case "$arch" in
    arm64|aarch64) arch=aarch64 ;;
    x86_64|amd64) arch=x86_64 ;;
    *) from_source "No prebuilt binary for $arch." ;;
esac
target="$arch-$os"
archive="knav-$target.tar.gz"

if [ "$version" = "latest" ]; then
    base="https://github.com/$repo/releases/latest/download"
else
    base="https://github.com/$repo/releases/download/$version"
fi

say "Downloading knav for $target ($version)..."
fetch "$base/$archive" "$archive" 2>/dev/null || from_source "No $version release for $target."
fetch "$base/$archive.sha256" "$archive.sha256" || die "couldn't download the checksum file"

expected=$(awk '{print $1}' "$archive.sha256")
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$archive" | awk '{print $1}')
else
    actual=$(shasum -a 256 "$archive" | awk '{print $1}')
fi
[ "$expected" = "$actual" ] || die "checksum mismatch, not installing"

tar xzf "$archive"
mkdir -p "$install_dir"
mv "knav-$target/knav" "$install_dir/knav"
chmod +x "$install_dir/knav"
finish
