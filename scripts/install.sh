#!/bin/sh
# Install the self-contained skit binary from GitHub Releases (no Python or uv needed).
#
#   curl -fsSL https://raw.githubusercontent.com/t41372/skit/main/scripts/install.sh | sh
#
# Environment knobs (all optional):
#   SKIT_VERSION         install this tag (e.g. v0.5.0) instead of the latest release
#   SKIT_INSTALL_DIR     target directory (default: ~/.local/bin)
#   SKIT_INSTALL_MIRROR  URL prefix prepended to every GitHub URL, for gh-proxy-style
#                        mirrors (mainland China):  SKIT_INSTALL_MIRROR=https://ghproxy.example/
#
# The script only ever writes the downloaded binary into SKIT_INSTALL_DIR; it touches no
# shell profile and never needs sudo. Integrity: the sha256 from the release's
# checksums.txt is verified before the binary is installed. That catches corrupt,
# truncated, or stale downloads — but checksums.txt travels through the same mirror, so
# it is NOT proof of authenticity against a hostile mirror: only use a mirror you trust,
# and for hard verification run `gh attestation verify <asset> --repo t41372/skit` (or
# compare the sha256 against a checksums.txt fetched without the mirror).
set -eu

REPO="t41372/skit"
INSTALL_DIR="${SKIT_INSTALL_DIR:-$HOME/.local/bin}"
MIRROR="${SKIT_INSTALL_MIRROR:-}"

say() { printf '%s\n' "$*" >&2; }
die() { say "install.sh: $*"; exit 1; }

os=$(uname -s)
arch=$(uname -m)
case "$os" in
  Linux)
    target="linux"
    # A musl userland (Alpine and friends) needs the musl build; the fixed ld-musl path
    # is the same probe skit itself uses.
    libc=""
    if [ -e /lib/ld-musl-x86_64.so.1 ] || [ -e /lib/ld-musl-aarch64.so.1 ]; then
      libc="-musl"
    fi
    ;;
  Darwin) target="darwin"; libc="" ;;
  *) die "unsupported OS: $os (Windows: download skit-windows-x86_64.exe from https://github.com/$REPO/releases)" ;;
esac
case "$arch" in
  x86_64|amd64) arch="x86_64" ;;
  aarch64|arm64) arch="$( [ "$target" = "darwin" ] && echo arm64 || echo aarch64 )" ;;
  *) die "unsupported architecture: $arch" ;;
esac
asset="skit-$target-$arch$libc"
[ "$target" = "linux" ] && [ "$arch" = "aarch64" ] && [ "$libc" = "-musl" ] \
  && die "no musl aarch64 binary yet — install with: pip install skit-cli (or uv tool install skit-cli)"

if [ -n "${SKIT_VERSION:-}" ]; then
  base="https://github.com/$REPO/releases/download/$SKIT_VERSION"
else
  base="https://github.com/$REPO/releases/latest/download"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

say "Downloading $asset ..."
curl -fsSL --proto '=https' -o "$tmp/$asset" "$MIRROR$base/$asset" \
  || die "download failed: $MIRROR$base/$asset"
curl -fsSL --proto '=https' -o "$tmp/checksums.txt" "$MIRROR$base/checksums.txt" \
  || die "download failed: $MIRROR$base/checksums.txt"

expected=$(awk -v a="$asset" '$2 == a { print $1 }' "$tmp/checksums.txt")
[ -n "$expected" ] || die "checksums.txt has no entry for $asset"
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$tmp/$asset" | awk '{ print $1 }')
else
  actual=$(shasum -a 256 "$tmp/$asset" | awk '{ print $1 }')
fi
[ "$actual" = "$expected" ] || die "sha256 mismatch for $asset (expected $expected, got $actual) — corrupt or tampered download; try again or drop SKIT_INSTALL_MIRROR"

mkdir -p "$INSTALL_DIR"
install -m 755 "$tmp/$asset" "$INSTALL_DIR/skit"
# A binary that installed but cannot run (glibc too old, noexec temp dir) must fail HERE,
# loudly — not report success and break at first real use.
if ver=$("$INSTALL_DIR/skit" --version 2>&1); then
  say "Installed: $INSTALL_DIR/skit ($ver)"
else
  say "$ver"
  die "installed binary does not run on this system — likely glibc older than 2.26 or a noexec temp dir; install with: uv tool install skit-cli (or pip install skit-cli)"
fi

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "Note: $INSTALL_DIR is not on your PATH — add it, e.g.:  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
