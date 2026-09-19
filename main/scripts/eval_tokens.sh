#!/usr/bin/env bash
# Lightweight token-wedge smoke metrics. Does not fail if optional CLIs are absent.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${TMPDIR:-/tmp}/cntx-eval-tokens-$$.jsonl"
PROMPTS=(
  "summarize src/lib.rs"
  "where is run_tool_loop defined?"
  "list rust files under src"
  "what does Mode::Plan do?"
  "read Cargo.toml briefly"
  "explain the edit sandbox"
  "find bash timeout defaults"
  "outline src/tools.rs"
  "how does compaction work?"
  "what providers are supported?"
)

echo "writing metrics to $OUT"
: >"$OUT"
if command -v cntx >/dev/null 2>&1; then
  BIN=cntx
elif [[ -x "$ROOT/target/debug/cntx" ]]; then
  BIN="$ROOT/target/debug/cntx"
else
  echo "cntx binary not found; build with cargo build first" >&2
  exit 0
fi

for p in "${PROMPTS[@]}"; do
  # JSONL usage lines when --jsonl is supported; ignore provider auth failures.
  if "$BIN" --no-interactive --jsonl --chat-only "$p" >>"$OUT" 2>/dev/null; then
    :
  else
    echo "{\"type\":\"skip\",\"prompt\":$(printf '%s' "$p" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')}" >>"$OUT"
  fi
done

echo "recorded $(wc -l <"$OUT") lines"
# Optional peer CLIs
for peer in claude codex opencode; do
  if command -v "$peer" >/dev/null 2>&1; then
    echo "peer available: $peer (not auto-run)"
  fi
done
