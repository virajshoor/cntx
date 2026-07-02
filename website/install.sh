#!/bin/sh
set -eu

if command -v brew >/dev/null 2>&1; then
  brew tap virajshoor/cntx
  brew install cntx
elif command -v cargo >/dev/null 2>&1; then
  echo "Building cntx from source (this may take a few minutes)..."
  tmpdir="$(mktemp -d)"
  git clone --depth 1 https://github.com/virajshoor/cntx.git "$tmpdir"
  cd "$tmpdir/main"
  cargo install --path .
  cd /
  rm -rf "$tmpdir"
else
  echo "Cntx needs Homebrew or Rust/Cargo to install." >&2
  echo "Install Homebrew from https://brew.sh or Rust from https://rustup.rs, then rerun this script." >&2
  exit 1
fi

echo
echo "Cntx installed."
echo "Next:"
echo "  cntx init"
echo "  cntx demo"
