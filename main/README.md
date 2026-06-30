# Cntx Code

Cntx Code is a BYOK, token-efficient AI coding assistant for the terminal. It is
inspired by modern agentic coding tools, but its central design goal is token
efficiency: keep the normal developer workflow while sending less unnecessary
context to model providers.

- Web: <https://cntxcode.com>
- Docs: <https://cntx.codes> (redirects to cntxcode.com)

## What Cntx Code Is For

- You choose your provider, endpoint, model, and API key.
- Cntx optimizes prompts before routing.
- Auto mode chooses a model based on optimized prompt size.
- Counsel mode uses multiple model families while keeping the evaluation prompt bounded.
- Built-in doc search (Context7) and built-in token saving (Headroom) ship as
  on-demand MCP servers; custom MCPs are supported too.
- An edit sandbox confines file edits to your project by default so the assistant
  cannot break the wider machine.
- API keys live in a gitignored runtime store that auto-creates on first boot.
- Aliases and a persistent default model keep daily usage simple.

## Current Status

Implemented:

- Rust `cntx` CLI binary
- OpenAI, Anthropic, OpenAI-compatible, Ollama Local, and Ollama Cloud provider adapters
- Custom provider presets defined in YAML
- Runtime API key store (`cntx api-key add/change/delete/list`)
- Endpoint create/change/remove/list/set-primary/import
- Model alias add/remove/list, and a persistent default model (`cntx model default`)
- Model cache with deprecation detection and refresh
- Automatic model routing by prompt size after optimization
- Counsel mode for token-efficient multi-model collaboration
- Prompt optimization and bounded project context selection
- Streaming chat adapters
- Interactive shell with slash commands
- Interactive markdown rendering, status prompt, and working-dot animation
- Session list/resume/export/import
- Skills stored in user or project config
- Edit sandbox with `--allow-write` and `--dangerously-disable-sandbox`
- Apply mode that writes `path=` fenced code blocks through the sandbox and
  prints a file checklist
- Built-in MCP servers (Context7 doc search, Headroom token saving) plus custom MCPs
- Permission modes: auto, counsel, allow, request-permission, file-only
- Unit tests for core behavior

Not complete yet:

- Fully autonomous multi-step tool loop
- Provider-specific tokenizers
- Semantic repository index
- Shell completion generation
- Full agent loop that calls MCP tools automatically

## Install

From source:

```bash
cargo install --path .
```

During development:

```bash
cargo run -- --help
```

## Quick Start

```bash
# Add an API key (stored in a gitignored runtime file)
cntx api-key add --provider anthropic --value sk-ant-...

# Create an endpoint
cntx endpoint --new \
  --name work \
  --provider anthropic \
  --api-key-env ANTHROPIC_API_KEY \
  --default-model claude-sonnet-4.5

cntx endpoint --set-primary work
cntx --refresh-models
cntx "explain this repository"
```

Pick a default model once and stop passing `--model`:

```bash
cntx model default claude-sonnet-4.5
cntx "write a focused test plan"
```

Use built-in doc search and token saving:

```bash
cntx mcp list
cntx mcp tools context7
cntx mcp tools headroom
```

Use the sandbox:

```bash
cntx sandbox
cntx --allow-write /Users/you/shared "refactor the shared utilities"
```

Use apply mode for real file writes:

```bash
cntx --apply --mode allow "create a small README section for this crate"
cntx
/apply
/checklist
```

In apply mode, Cntx asks the model for complete fenced code blocks annotated with
`path=...`, writes them through the sandbox, and keeps the last file checklist
available in interactive chat.

## Documentation

- [Project explanation](EXPLAIN.md)
- [Growth, X, the gold tick, build, publish, monetization](HELP.md)
- [API keys](docs/api-keys.md)
- [Apply mode](docs/apply.md)
- [Doc search and token saving (MCP)](docs/mcp.md)
- [Custom providers](docs/custom-providers.md)
- [Sandbox](docs/sandbox.md)
- [Provider setup](docs/providers.md)
- [Ollama Cloud and Pro](docs/ollama-cloud.md)
- [Command reference](docs/commands.md)
- [Model routing](docs/routing.md)
- [Configuration](docs/configuration.md)
- [Skills](docs/skills.md)
- [Modes](docs/modes.md)
- [Sessions](docs/sessions.md)
- [API references](docs/api-references.md)
- [Troubleshooting](docs/troubleshooting.md)

## Verification

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
```

## License

MIT
