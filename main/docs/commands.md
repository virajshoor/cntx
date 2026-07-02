# Command Reference

## Prompting

```bash
cntx "fix the failing tests"
cntx --model fast "summarize this module"
cntx --endpoint local "review the latest diff"
cntx --endpoint ollama-pro --model deepseek-v4-pro:cloud "review architecture risks"
```

Running `cntx` without a prompt opens the interactive shell. Assistant responses
and slash-command help render markdown in the terminal, so bold text, inline code,
lists, and code blocks display as formatted output rather than raw punctuation.

Interactive commands:

```text
/help
/status
/apply
/dry-run
/checklist
/sandbox
/models
/endpoints
/exit
```

## Global Options

```bash
cntx --model <MODEL_OR_ALIAS>
cntx --endpoint <ENDPOINT_NAME>
cntx --mode auto|counsel|allow|request-permission|file-only
cntx --refresh-models
cntx --docs                          # open packaged interactive docs
cntx --no-interactive "single prompt"
cntx --allow-write <PATH>            # extend the edit sandbox (repeatable)
cntx --apply                         # write path= fenced blocks through sandbox
cntx --dry-run                       # preview apply-mode writes without writing
cntx --dangerously-disable-sandbox "edit anywhere"
```

Prompts automatically include a small amount of bounded project context when it
is useful. Use `@path/to/file` in a prompt to force an explicit file excerpt into
the request while keeping the prompt capped.

Counsel mode:

```bash
cntx --mode counsel "refactor the endpoint code with minimal churn"
```

## First Run And Diagnostics

```bash
cntx init --yes --provider anthropic --name work
cntx init --provider ollama-cloud --default-model deepseek-v4-pro:cloud
cntx doctor
cntx doctor --fix
cntx doctor --json
cntx demo
cntx --docs
```

`init` creates or updates an endpoint, sets it primary, installs built-in provider
presets, and optionally stores a runtime key. `doctor --fix` creates missing local
config files, ensures the secrets store exists, installs safe provider presets, and
sets obvious defaults when only one endpoint exists.

`cntx --docs` opens the packaged docs browser. It is compiled into the binary, so
installed users can read the README, explanation, command reference, apply-mode
guide, provider docs, sandbox docs, and troubleshooting from the terminal.

## Benchmark And Cost Preview

```bash
cntx bench "summarize this repository and propose the smallest safe patch"
cntx bench --json "summarize this repository and propose the smallest safe patch"
```

`bench` does not call a model. It shows user prompt size, bounded project context,
optimized characters, estimated input tokens, duplicate lines removed, routed
model if one can be chosen, and a rough request cost for common model families.

## API Keys

```bash
cntx api-key add    --provider anthropic --value sk-...
cntx api-key change --provider openai --value sk-...
cntx api-key delete --provider anthropic
cntx api-key list
```

Keys are stored in a gitignored runtime file; see [API keys](api-keys.md).

## Endpoints

```bash
cntx endpoint --new --name work --provider anthropic --api-key-env ANTHROPIC_API_KEY
cntx endpoint --change work --default-model claude-sonnet-4.5
cntx endpoint --remove work
cntx endpoint --list
cntx endpoint --set-primary work
cntx endpoint --new --name gw --from-preset gateway   # from a custom provider preset
cntx endpoint --import providers.yaml
```

Provider values: `open-ai`, `anthropic`, `open-ai-compatible`, `ollama-local`,
`ollama-cloud`.

## Custom Providers

```bash
cntx provider gallery
cntx provider install-preset openrouter
cntx provider add --name gateway --kind open-ai-compatible \
  --base-url https://gateway.example.com/v1 --api-key-env GATEWAY_API_KEY
cntx provider add --file providers.yaml
cntx provider list
cntx provider use gateway
cntx provider remove gateway
```

See [Custom providers](custom-providers.md).

## Project Memory

```bash
cntx memory add prefer focused patches and keep docs updated
cntx memory show
cntx memory path
```

Project memory is stored in `.cntx/memory.md` under the current project root.

## Doc Search And Token Saving (MCP)

```bash
cntx mcp list
cntx mcp tools context7      # built-in doc search
cntx mcp tools headroom      # built-in token saving
cntx mcp add filesystem npx --arg -y --arg @modelcontextprotocol/server-filesystem
cntx mcp add --file mcp-servers.yaml
cntx mcp enable filesystem
cntx mcp disable filesystem
cntx mcp remove filesystem
```

See [Doc search and token saving](mcp.md).

## Models And Aliases

```bash
cntx model add claude-sonnet-4.5 --name fast
cntx model add deepseek-v4-pro:cloud --name ollama-pro-coder --endpoint ollama-pro
cntx model remove fast
cntx model default claude-sonnet-4.5     # set the persistent default model
cntx model default --unset               # clear it
cntx model list
cntx model refresh
cntx --refresh-models
```

Aliases are stored separately from the refreshed model cache, so refreshes preserve
aliases. `cntx model default <name>` picks the model used when no `--model` override
applies and routing does not select one.

## Sandbox

```bash
cntx sandbox
cntx sandbox --yaml
cntx --apply --mode allow "write the docs update"
cntx --apply --dry-run --mode allow "preview the docs update"
```

See [Sandbox](sandbox.md).

## Config

```bash
cntx config init
cntx config path
cntx config show
```

## Sessions

```bash
cntx session list
cntx session resume          # latest session
cntx session resume <id>
cntx session export <id> session.json
cntx session import session.json
```

## Completions

```bash
cntx completions zsh
cntx completions bash
cntx completions fish
```

## Skills

```bash
cntx skill list
cntx skill new repository-standards "Apply this repository's coding standards"
cntx skill show repository-standards
```

## Diagnostics

```bash
cntx doctor
cntx doctor --fix
```
