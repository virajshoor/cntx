# Small Business Quickstart

Cntx Code is a terminal assistant that reads, writes, and edits files and runs
commands for you. You bring your own AI provider key (BYOK); usage is billed
by that provider, not by cntx.

## 1. Install (macOS or Linux)

```bash
cargo install cntx
```

Requires a C compiler and a Rust toolchain. Check with:

```bash
cntx doctor
```

## 2. Simplest path: OpenCode Go ($10/month subscription)

```bash
cntx init --provider opencode-go
cntx --refresh-models
cntx "explain this repository"
```

When prompted, paste your Go API key (or set `OPENCODE_GO_API_KEY`). This is
the recommended path for teams that want one bill and no per-key management.

## 3. Any other provider (Anthropic, OpenAI, Ollama, ...)

```bash
cntx init --provider anthropic
cntx --refresh-models
cntx "summarize the README"
```

`cntx init` creates your endpoint, stores the key, and sets a default model.
Later key changes:

```bash
cntx api-key add --provider anthropic
```

## 4. Your first real tasks

```bash
cntx "add pagination to the users endpoint and run the tests"
cntx --mode manual-approve "show me exactly what you would change"
```

- Default mode `auto-approve` asks before writes and shell commands.
- `/mode` inside a session shows and changes the mode.
- `/compact` summarizes older turns when context fills up.
- `cntx --dry-run "..."` previews without changing anything.

## 5. Who pays for what

- Cntx itself is free (MIT). You pay your AI provider for usage.
- Estimate before a big run: `cntx bench "migrate the database layer"`.
- Session cost estimates: `/cost` inside a session.

## Next steps

- [Team Admin Guide](team-admin-guide.md) — roll out to staff.
- [Security Overview](security-overview.md) — what data leaves the machine.
- [Troubleshooting](troubleshooting.md) — endpoint, key, and model fixes.
