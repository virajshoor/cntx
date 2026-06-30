# Cntx Code Workspace Instructions

These instructions apply to all changes under this `main/` workspace.

## Efficiency First

Cntx Code is a token-efficient and memory-conscious coding assistant. Every change
should preserve or improve:

- minimal prompt size
- bounded memory usage
- streaming behavior
- provider-agnostic abstractions
- fast CLI startup
- clear tests for routing, optimization, provider behavior, the sandbox, and the API
  key store

Avoid adding unbounded context reads, duplicated prompt payloads, large cached
provider responses, or hardcoded full model lists.

## Safety First

The edit sandbox is a core feature. Never bypass path containment silently. New
tools that write files must call the sandbox before performing the write. Only
`--dangerously-disable-sandbox` removes containment, and it must remain opt-in.

API keys never belong in source. Use the runtime key store (`src/api_keys.rs`) or
endpoint `api_key_env`. Never add a real key to a committed file.

## Counsel Mode

Counsel mode is a token-efficient multi-model strategy:

- Haiku-class models evaluate and classify the request.
- Sonnet-class models handle small changes.
- Opus-class models handle refactors and larger redesigns.

For non-Anthropic providers, use equivalent small, medium, and large model families.
Keep the evaluation prompt bounded and pass only a short evaluator note to the
worker model.

## Documentation

When changing user-visible behavior, update:

- `README.md`
- `EXPLAIN.md`
- relevant files under `docs/`
- `HELP.md` for growth, publishing, and monetization context

## Verification

Run these checks before handoff:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
```