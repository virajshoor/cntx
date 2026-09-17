# Security Overview

What data leaves the machine, where secrets live, and what the sandbox does
and does not guarantee.

## Data flow

1. Your prompt plus assembled project context (memory, instructions, git
   summary, `@file` references) is sent to your configured AI provider over
   HTTPS. Prompts and file contents go to that third party by design (BYOK).
2. Tool results (file contents, command output) return to the provider as
   follow-up messages until the model answers.
3. Sessions, transcripts, and config stay on the local machine under the
   config dir (`~/Library/Application Support/cntxcode/` on macOS,
   `~/.config/cntxcode/` on Linux, or `$CNTX_CONFIG_DIR`).

Nothing is sent to cntx infrastructure: there is none. Model catalog refresh
and MCP subprocesses (`npx`, doc search) make their own network calls as
documented in [Providers](providers.md) and [MCP](mcp.md).

## Secrets

- `cntx api-key add --provider <name>` stores keys in `secrets.yaml` (file
  mode `0600` on Unix), auto-created on first boot. List without values:
  `cntx api-key list` shows masked tails only.
- Keys may also come from env vars (`api_key_env`) or inline `api_key` in
  `config.yaml`. Env vars override the store; inline keys are discouraged
  because `config.yaml` is easier to copy around.
- Reads, grep, and glob refuse known secret/credential filenames so the model
  cannot pull credentials through a differently-named path.
- There is no vault/KMS integration, rotation, or expiry. Rotate at the
  provider and re-add.

## Sandbox (application policy, not OS isolation)

- File writes are confined to the project root; `..` traversal, dangling
  symlinks, and outside-root writes are denied (`--allow-write <root>`
  extends the boundary explicitly).
- Approval modes decide when a human is asked: `auto-approve` writes code
  freely and asks before shell commands (`y` once, `ya` for the session,
  `n` declines), `all-approve` runs permitted tools without prompting,
  `manual-approve` asks before everything, `file-only` denies shell/network.
- An approved shell command runs with your user privileges and can reach the
  wider machine, like any command you type yourself.
  `--dangerously-disable-sandbox` removes containment entirely; never use it
  on untrusted prompts.
- Shell execution is bounded: 60s default timeout (up to 600s), process-group
  termination on timeout/cancel, 10 MiB output cap per stream.

## Reporting

Security issues: open a private report via the repository's security tab
rather than a public issue. Include version (`cntx --version`), OS, and
reproduction steps without real keys.
