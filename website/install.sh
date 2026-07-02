#!/bin/sh
set -eu

if command -v cargo >/dev/null 2>&1; then
  cargo install cntx
elif command -v brew >/dev/null 2>&1; then
  brew tap virajshoor/cntx
  brew install cntx
else
  echo "Cntx needs Rust/Cargo or Homebrew to install." >&2
  echo "Install Rust from https://rustup.rs or Homebrew from https://brew.sh, then rerun this script." >&2
  exit 1
fi

echo
echo "Cntx installed."
echo "Next:"
echo "  cntx init"
echo "  cntx demo"
