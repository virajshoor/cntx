# Cntx Code Explanation

## Project Idea

Cntx Code is a Bring Your Own Keys AI coding assistant for the terminal. The goal is to give developers a Claude Code-like workflow while reducing token waste. The project lives at <https://cntxcode.com> (with <https://cntx.codes> redirecting to it).

Most coding assistants become expensive because they send too much repeated, stale, or irrelevant context. Cntx Code treats token efficiency as a core product feature instead of a manual prompt trick. The assistant should feel natural to use, but internally it should optimize context, preserve useful memory, route to the right model size, and keep request payloads lean.

## Product Philosophy

Cntx Code is designed around these principles:

- BYOK: users control their API keys, endpoints, and models
- provider agnostic: adding a provider should mean adding an adapter or a YAML preset
- token efficient by default: optimization should happen automatically
- Counsel mode must stay token efficient even though it can use multiple models
- safe by default: file edits are confined to the project sandbox
- built-in capabilities ship ready: doc search and token saving are available out of the box
- fast CLI experience: startup and interaction should feel native
- extensible architecture: future providers, skills, tools, MCPs, and plugins should fit cleanly
- clear configuration: endpoints, aliases, routing, MCPs, and UI settings should be inspectable

## What Has Been Built

The repository contains a Rust CLI application named `cntx`.

Implemented systems:

- CLI command parsing with `clap`
- standard user config storage
- endpoint management
- provider abstraction and adapters
- custom provider presets defined in YAML
- runtime API key store (gitignored, auto-created on first boot)
- model aliases and a persistent default model
- model refresh and cache
- auto routing by optimized prompt size
- Counsel mode for token-efficient model collaboration
- prompt optimization
- memory-conscious context selection primitives
- streaming response handling
- interactive shell with markdown rendering, `/status`, slash commands, and a
  working-dot animation during model calls
- session storage, listing, export, and import
- skill storage and listing
- edit sandbox with path-bounded write containment
- apply mode that writes model-proposed `path=` fenced code blocks through the
  sandbox and prints a reusable checklist
- built-in MCP servers (Context7 doc search, Headroom token saving) plus custom MCPs
- permission modes
- docs and tests
- packaged interactive docs browser via `cntx --docs`

## Providers

Supported providers:

- OpenAI
- Anthropic
- OpenAI-compatible APIs
- Ollama Local
- Ollama Cloud

The provider system is adapter-based. A future provider should plug in by implementing the provider adapter behavior for listing models and streaming chat responses. For gateways that only need different base URLs, headers, or request paths, a [custom provider preset](docs/custom-providers.md) reuses an existing adapter without new code.

## Ollama Cloud Pro Support

Cntx supports Ollama Cloud through the `ollama-cloud` provider.

Ollama Cloud Free, Pro, Max, and Team use the same direct API host:

```text
https://ollama.com
```

Access to subscription models is controlled by the Ollama API key and account plan.

Cntx now supports endpoint metadata for:

- `free`
- `pro`
- `max`
- `team`

Example:

```bash
cntx endpoint --new \
  --name ollama-pro \
  --provider ollama-cloud \
  --ollama-cloud-plan pro \
  --default-model deepseek-v4-pro:cloud
```

When `ollama-cloud` is selected and no key is specified, Cntx defaults to:

```text
OLLAMA_API_KEY
```

## Token Optimization

The current optimizer:

- normalizes natural-language whitespace
- removes duplicate natural-language lines
- preserves code fences
- collapses excessive blank lines
- estimates prompt tokens

Project context selection is bounded so large files do not create unbounded memory pressure. It reads a capped prefix and stores short excerpts.

Counsel mode is also token-efficient. It does not send the complete optimized prompt to every model. Instead, it sends a bounded preview to the evaluator model, then sends the optimized prompt plus a short evaluator note to the selected worker model.

## Model Routing

Auto mode uses the primary endpoint.

Routing is not based on semantic complexity. It is based on prompt length after optimization:

- small prompt -> small/light model
- medium prompt -> balanced model
- large prompt -> stronger/larger model

Examples:

- Anthropic: Haiku, Sonnet, Opus
- OpenAI-compatible: mini/nano, default, pro/large/max
- Ollama: parameter size and usage metadata
- Ollama Cloud Pro: names such as `deepseek-v4-pro:cloud` rank as large candidates

Users can still override routing with:

```bash
cntx --model <model-or-alias>
```

## Counsel Mode

Counsel mode is a working mode in addition to Auto.

It uses a mix of models:

- Haiku-class models evaluate the request and identify risk.
- Sonnet-class models handle small changes.
- Opus-class models handle refactors and larger redesigns.

For non-Anthropic providers, Cntx maps those roles to equivalent small, medium, and large model families.

Example:

```bash
cntx --mode counsel "refactor the provider routing while preserving token efficiency"
```

Counsel mode keeps the evaluator prompt bounded and passes only a concise evaluator note forward, so the multi-model flow does not casually multiply token usage.

## Configuration

Configuration is stored in the user config directory unless `CNTX_CONFIG_DIR` is set.

Useful commands:

```bash
cntx config path
cntx config show
cntx doctor
```

## Packaged Docs Browser

The published crate now includes the public README, docs directory, changelog,
license, and `EXPLAIN.md`. `EXPLAIN.md` was removed from the package exclusion list
so the explanation can ship with the crate.

Users can open an interactive terminal docs browser with:

```bash
cntx --docs
```

The browser renders markdown pages from compile-time embedded docs, so it works
after `cargo install cntx` without needing source files beside the binary.

## Sessions

Interactive sessions are stored in the config directory. Sessions can be listed, resumed, exported, or imported.

```bash
cntx session list
cntx session resume <id>
cntx session export <id> session.json
cntx session import session.json
```

## Skills

Skills are reusable instructions for project knowledge, workflows, coding standards, or architecture notes.

```bash
cntx skill new repository-standards "Apply this repository's coding standards"
cntx skill list
cntx skill show repository-standards
```

## API Keys

API keys never live in source. They are stored in a gitignored runtime file
(`secrets.yaml`) inside the config directory, created automatically on first boot
with `0600` permissions. Keys are added, changed, deleted, and listed with
`cntx api-key`. At request time, an endpoint resolves its key from its inline key,
then its `api_key_env`, then the runtime store keyed by provider. This makes
`cntx api-key add --provider anthropic` enough to run any Anthropic endpoint.

See [API keys](docs/api-keys.md).

## Doc Search And Token Saving (MCP)

Cntx Code ships two built-in capabilities as on-demand MCP servers:

- Context7, surfaced as built-in documentation search (`resolve-library-id`, `query-docs`)
- Headroom, surfaced as built-in token saving (`headroom_compress`, `headroom_retrieve`, `headroom_stats`)

Servers run as local stdio subprocesses and are spawned only when you ask for them,
so ordinary prompts stay fast. Custom MCP servers can be added from YAML or the CLI.

See [Doc search and token saving](docs/mcp.md).

## Edit Sandbox

File edits are confined to the project root by default. Writes outside the sandbox
are denied, even in `allow` mode. Shell and network access are gated by the active
permission mode. The sandbox is widened with `--allow-write` and disabled with
`--dangerously-disable-sandbox`.

Apply mode uses that sandbox today. When `--apply` is enabled, Cntx prepends a
file-output instruction, parses fenced blocks annotated with `path=`, `file=`, or
`filename=`, writes complete file contents inside allowed roots, and prints a
checklist of written, blocked, and outside-sandbox paths. In interactive mode,
`/checklist` shows the last apply result.

See [Sandbox](docs/sandbox.md).

## Custom Providers

A custom provider is a YAML preset that reuses an existing adapter family
(OpenAI-compatible, Anthropic, or Ollama) for a specific gateway, with optional
custom `models_path` and `chat_path` overrides. An endpoint can be created from a
preset with `cntx endpoint --new --from-preset <name>` or `cntx provider use <name>`.

See [Custom providers](docs/custom-providers.md).

## Default Model

`cntx model default <model-or-alias>` sets a persistent default used when no
`--model` override applies and routing does not select a model. Clear it with
`cntx model default --unset`.

## What Is Still Missing

The project is a strong foundation, but it is not yet a complete premium coding agent.

Important future work:

- fully autonomous multi-step tool execution loop
- provider-specific tokenizers
- semantic repository indexing
- richer syntax themes and terminal layout controls
- shell completions
- setup wizard
- cost and latency dashboards
- benchmark command
- request and response caching
- plugin API
- deeper Git integration

## How To Use It Today

Build the CLI:

```bash
cargo build
```

Add an API key (stored in a gitignored runtime file):

```bash
cntx api-key add --provider anthropic --value sk-ant-...
```

Create a provider endpoint:

```bash
cntx endpoint --new --name work --provider anthropic --api-key-env ANTHROPIC_API_KEY
cntx endpoint --set-primary work
```

Refresh models:

```bash
cntx --refresh-models
```

Run a prompt:

```bash
cntx "explain the project"
```

Use a specific model or a persistent default:

```bash
cntx --model deepseek-v4-pro:cloud "review this module"
cntx model default claude-sonnet-4.5
```

Use Counsel mode:

```bash
cntx --mode counsel "refactor the provider routing"
```

Use built-in doc search and token saving:

```bash
cntx mcp tools context7
cntx mcp tools headroom
```

Check the sandbox:

```bash
cntx sandbox
```

Open interactive mode:

```bash
cntx
```

## Current Verification

The project has unit tests covering config round trips, provider model parsing, prompt optimization, context selection, streaming parser behavior, model routing, and Ollama Cloud plan handling.

Run checks:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
```
