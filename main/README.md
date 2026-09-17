# Cntx Code

Cntx Code is a BYOK, token-efficient AI coding assistant for the terminal. Its
core is written in C (C17) with Rust for transport, terminal, and
serialization, so normal prompts reliably create and edit files, run commands,
and keep context across turns.

- Web: <https://cntxcode.com>
- Docs: <https://cntxcode.com/docs>
- Source: <https://github.com/virajshoor/cntx>

## What Cntx Code Is For

- You choose your provider, endpoint, model, and API key.
- Tool mode is on by default: prompts can read, write, and edit files and run
  shell commands, with approval modes controlling what asks first.
- An edit sandbox confines file writes to your project by default; symlink and
  traversal escapes are rejected. Shell commands are policy-gated but are not
  OS-level isolation.
- Sessions persist tool results, decisions, and goal state; `/compact`
  summarizes older turns without changing the session id; `/goal` runs bounded,
  resumable, goal-oriented work.
- OpenCode Go works as a first-class subscription provider.
- API keys live in a gitignored runtime store that auto-creates on first boot.
- Auto mode chooses a model based on optimized prompt size; counsel mode keeps
  multi-model evaluation bounded.

## Install

Requires a C compiler (cc/clang/gcc) and a Rust toolchain; targets are macOS
and Linux.

From crates.io (recommended):

```bash
cargo install cntx
```

From source:

```bash
git clone https://github.com/virajshoor/cntx.git
cd cntx/main
cargo install --path .
```

## OpenCode Go quick start

[OpenCode Go](https://opencode.ai/docs/go/) is a $10/month subscription for
open coding models. Requests use the client user agent `cntx/<version>` and a
stable `x-opencode-session` id per conversation.

```bash
cntx api-key add --provider opencode-go
cntx provider install-preset opencode-go
cntx provider use opencode-go
cntx --refresh-models
cntx "explain this repository"
```

The key resolves from `OPENCODE_GO_API_KEY` or the runtime store; a Go endpoint
never borrows another provider's key. See
[docs/opencode-go.md](docs/opencode-go.md) for protocol details and limits.

## Any other provider

```bash
cntx api-key add --provider anthropic
cntx endpoint --new --name work --provider anthropic
cntx endpoint --set-primary work
cntx --refresh-models
cntx "explain this repository"
```

OpenAI, Anthropic, OpenAI-compatible gateways, Ollama Local, Ollama Cloud, and
YAML-defined custom presets are supported: [docs/providers.md](docs/providers.md).

## Working session example

```text
$ cntx
cntx › work/glm-5.3-flash auto-approve sandbox

> /goal add pagination to the users endpoint and run the tests
goal started (Ctrl+C pauses; /goal pause also works)
~ reading src/users.rs
~ [ok] reading src/users.rs
~ editing src/users.rs
~ [ok] editing src/users.rs
~ running: cargo test users
~ [ok] running: cargo test users
goal marked completed. Evidence recorded: cargo test users: 9 passed

> /model glm-5.3
model set to glm-5.3 for this session
> /mode manual-approve
mode: manual-approve - ask before every tool, file, or shell operation.
> /compact
compacted 14 messages (session 7f3c... unchanged)
```

## Approval modes

| Operation | auto-approve (default) | all-approve | manual-approve | file-only | counsel |
| --- | --- | --- | --- | --- | --- |
| Explicit read/glob/grep | Allow | Allow | Ask | Allow | Allow |
| In-root write/edit | Ask | Allow | Ask | Allow | Ask |
| Shell command | Ask | Allow | Ask | Deny | Ask |
| Outside-root write | Deny | Deny | Deny | Deny | Deny |

`--mode all-approve` executes permitted tools without prompts; file containment
still applies. `--dry-run` blocks mutations and shell execution.
[docs/modes.md](docs/modes.md) has full semantics.

## Documentation

- [Quickstart for Teams](docs/small-business-quickstart.md)
- [Team Admin Guide](docs/team-admin-guide.md)
- [Security Overview](docs/security-overview.md)
- [Enterprise Readiness](docs/enterprise-readiness.md)
- [Changelog](CHANGELOG.md)
- [Project explanation](EXPLAIN.md)
- [Command reference](docs/commands.md)
- [Approval modes](docs/modes.md)
- [Goals](docs/goals.md)
- [OpenCode Go](docs/opencode-go.md)
- [Sessions](docs/sessions.md)
- [API keys](docs/api-keys.md)
- [Apply mode](docs/apply.md)
- [Doc search and token saving (MCP)](docs/mcp.md)
- [Custom providers](docs/custom-providers.md)
- [Sandbox](docs/sandbox.md)
- [Provider setup](docs/providers.md)
- [Ollama Cloud and Pro](docs/ollama-cloud.md)
- [Model routing](docs/routing.md)
- [Configuration](docs/configuration.md)
- [Skills](docs/skills.md)
- [API references](docs/api-references.md)
- [Troubleshooting](docs/troubleshooting.md)

## Architecture and verification

- **C core** (`csrc/`): permission decisions, tool validation, file tools,
  bounded command execution, goal state machine, context budgets, routing
  classification, Go protocol selection.
- **Rust** (`src/`): CLI, terminal, HTTP/TLS streaming, JSON/YAML, key store,
  sessions, and the safe FFI wrapper (`src/core.rs`).
- **Shell** (`scripts/verify.sh`): repeatable verification.

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
sh scripts/verify.sh   # C sanitizer tests + package checks + temp-root install
```

## Known limitations

- Models call tools through a text `<tool>{...}</tool>` protocol, not
  provider-native tool-call APIs.
- Token counts are estimates, not provider tokenizers.
- MCP servers run on demand (`cntx mcp tools <name>`), not automatically.
- OpenCode Go protocol handling is verified against a local HTTP mock, not an
  authenticated live subscription.
- Windows is not supported.

## License

MIT

---

<p align="center"><sub>Inspired by <a href="https://github.com/cntx-ai/headroom">Headroom</a> — context compression for AI agents.</sub></p>
