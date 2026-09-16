<div align="center">

# CNTX

### A BYOK AI coding assistant for your terminal — in C and Rust.

Create and edit files, run and verify commands, and keep context across turns
without sending your code to a hosted agent backend.

[![CI](https://github.com/virajshoor/cntx/actions/workflows/ci.yml/badge.svg)](https://github.com/virajshoor/cntx/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](#license)

[Install](#installation) · [OpenCode Go](#opencode-go) · [Docs](https://cntxcode.com/docs) · [Source](https://github.com/virajshoor/cntx)

</div>

---

## What it does

1. **Create and edit files** — real writes through an edit sandbox, not just
   suggested patches.
2. **Execute and verify** — shell commands with timeouts, bounded output, and
   process-group cleanup, so the assistant can run your tests instead of
   guessing.
3. **Retain context and goals** — sessions persist tool results and decisions;
   `/compact` summarizes older turns; `/goal` drives bounded, resumable,
   goal-oriented work.

## Installation

Requires a C compiler (cc/clang/gcc) and Rust — the CLI links a C17 core.

```bash
cargo install cntx
```

From source:

```bash
git clone https://github.com/virajshoor/cntx.git
cd cntx/main
cargo install --path .
```

Supported targets: macOS and Linux.

## Quick start with OpenCode Go

[OpenCode Go](https://opencode.ai/docs/go/) is a $10/month subscription that
gives access to popular open coding models (GLM, Kimi, DeepSeek, MiniMax, Qwen,
GPT 5.6 Luna, Grok, and more):

```bash
cntx api-key add --provider opencode-go     # paste your Go key once
cntx provider install-preset opencode-go
cntx provider use opencode-go
cntx --refresh-models
cntx "explain this repository"
```

Details: [docs/opencode-go.md](main/docs/opencode-go.md). Any other provider
works too — see [provider setup](main/docs/providers.md).

## Terminal example

```text
$ cntx
cntx › work/glm-5.3-flash auto-approve sandbox
Type `/help` for commands.

> /goal fix the failing auth tests and run them
goal started (Ctrl+C pauses; /goal pause also works)
~ reading src/auth.rs
~ [ok] reading src/auth.rs
~ editing src/auth.rs
~ [ok] editing src/auth.rs
~ running: cargo test auth
~ [ok] running: cargo test auth
<tool>{"name":"goal_update","arguments":{"status":"completed","progress":"tests pass","evidence":"cargo test auth: 12 passed"}}</tool>
goal marked completed. Evidence recorded: cargo test auth: 12 passed

> /effort high
effort: high - investigate thoroughly and verify after changes.
> /mode manual-approve
mode: manual-approve - ask before every tool, file, or shell operation.
```

## Approval modes

Selectable with `/mode <name>` or Shift+Tab cycling. `Ask` means the human
must answer `y`/`yes`; denial is never retried or worked around.

| Operation | auto-approve (default) | all-approve | manual-approve | file-only | counsel |
| --- | --- | --- | --- | --- | --- |
| Explicit read/glob/grep | Allow | Allow | Ask | Allow | Allow |
| In-root write/edit | Ask | Allow | Ask | Allow | Ask |
| Shell command | Ask | Allow | Ask | Deny | Ask |
| Outside-root write | Deny | Deny | Deny | Deny | Deny |

The boundary between direct file tools and the shell: file writes are
path-contained by the sandbox (symlink and traversal escapes are rejected).
Shell commands are **application policy, not OS isolation** — an approved
command can access the wider machine, exactly as one you typed yourself could.
Outside-root writes need `--allow-write <root>`; containment removal needs
explicit `--dangerously-disable-sandbox`.

## Commands and sessions

| Command | Effect |
| --- | --- |
| `/mode [name]` | Show or switch the approval mode for this session |
| `/model [id]` | Show/override the model; `/model auto` restores routing |
| `/models` | List cached models grouped by endpoint |
| `/effort [low\|medium\|high]` | Investigation and verification depth |
| `/goal <objective>` | Start bounded goal work (default 50 steps per batch) |
| `/goal`, `/goal resume\|pause\|cancel` | Inspect / continue / pause / cancel |
| `/resume [id]` | Reload a session and continue interactively |
| `/compact` | Summarize older turns; session id stays the same |
| `/clear` | Fresh session; endpoint/model/mode/effort carry over |
| `/dry-run` | Block mutations and shell execution |
| `/cost`, `/status`, `/sandbox`, `/theme` | Session diagnostics |

Tool results, assistant tool requests, and goal state are persisted after every
step, so a follow-up turn — or `/resume` in a new shell — keeps the execution
history. Compaction keeps the summary, goal, decisions, and recent turns while
omitting covered old messages from provider requests. Token counts are
estimates; goals stop cleanly on step limits, denials, blockers, or Ctrl+C and
never loop unboundedly.

## Architecture

- **C (C17 core, `main/csrc/`)** — approval-mode decisions, tool validation,
  file tools, bounded command execution, goal state machine, context budgets,
  routing classification, OpenCode Go protocol selection.
- **Rust (`main/src/`)** — CLI parsing, terminal, HTTP/TLS streaming, JSON/YAML,
  key store, sessions, and the safe FFI wrapper (`src/core.rs`).
- **Shell (`main/scripts/verify.sh`)** — repeatable verification.

Contributor checks:

```bash
cd main
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
sh scripts/verify.sh   # adds C sanitizer tests + clean temp-root install
```

## Known limitations

- The model calls tools through a text `<tool>{...}</tool>` protocol rather
  than provider-native tool-call APIs; malformed blocks are rejected with
  correction feedback instead of silent success.
- Token counts are estimates (chars/4), not provider tokenizers.
- MCP servers (Context7, Headroom) are invoked manually with `cntx mcp tools`,
  not automatically in the loop.
- OpenCode Go protocol handling is verified against a local HTTP mock;
  no authenticated live subscription test was run.
- Windows is not supported.

## License and links

MIT. Documentation lives in [`main/docs/`](main/README.md#documentation), the
[website](https://cntxcode.com), and the [changelog](main/CHANGELOG.md).

<div align="center">

**Fast AI. Minimal overhead. Right from your terminal.**

</div>
