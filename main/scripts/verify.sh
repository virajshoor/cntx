#!/bin/sh
# Reproducible Rust + C verification for Cntx.
#
# Runs the required checks from any location, uses `set -eu` so any check
# failure aborts, and cleans only the temporary directories it creates.
# Rust checks run from main/; C checks compile the checked-in sources with
# AddressSanitizer/UndefinedBehaviorSanitizer where supported.
set -eu

REPO_ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/cntx-verify-XXXXXX")
cleanup() {
    rm -rf "$WORK"
}
trap cleanup EXIT

echo "==> C self-test (sanitizers where supported)"
SAN=""
if clang -fsanitize=address,undefined -x c -o "$WORK/probe" - <<'EOF' >/dev/null 2>&1
int main(void) { return 0; }
EOF
then
    SAN="-fsanitize=address,undefined"
fi
clang -std=c17 -D_POSIX_C_SOURCE=200809L -Wall -Wextra -Wpedantic $SAN \
    -I "$REPO_ROOT/main/csrc" \
    "$REPO_ROOT/main/csrc/agent.c" \
    "$REPO_ROOT/main/csrc/context.c" \
    "$REPO_ROOT/main/csrc/permissions.c" \
    "$REPO_ROOT/main/csrc/routing.c" \
    "$REPO_ROOT/main/csrc/tools.c" \
    "$REPO_ROOT/main/tests/c_selftest.c" \
    -o "$WORK/cntx-selftest"
"$WORK/cntx-selftest"

echo "==> Rust checks (from main/)"
cd "$REPO_ROOT/main"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
cargo package --list >/dev/null

echo "==> Clean install into a temporary root"
cargo install --path "$REPO_ROOT/main" --root "$WORK/cntx-root" --quiet
test -x "$WORK/cntx-root/bin/cntx"
# The system-installed cntx is untouched; verify only the temporary binary.
"$WORK/cntx-root/bin/cntx" --version

echo "verify.sh: all checks passed"
