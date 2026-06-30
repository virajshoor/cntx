# Configuration

Cntx Code uses the platform config directory by default:

```bash
cntx config path
```

For isolated runs:

```bash
export CNTX_CONFIG_DIR=/tmp/cntx-test
```

## Config Shape

```yaml
version: 1
primary_endpoint: ollama-pro
default_model: claude-sonnet-4.5
endpoints:
  ollama-pro:
    name: ollama-pro
    provider: ollama-cloud
    api_key: null
    api_key_env: OLLAMA_API_KEY
    base_url: https://ollama.com
    default_model: deepseek-v4-pro:cloud
    custom_headers: {}
    timeout_secs: 120
    metadata: {}
    ollama_cloud:
      plan: pro
      subscription_models: true
aliases: {}
custom_providers:
  gateway:
    name: gateway
    kind: open-ai-compatible
    base_url: https://gateway.example.com/v1
    api_key_env: GATEWAY_API_KEY
    default_model: gpt-4o
    headers: {}
    models_path: null
    chat_path: null
mcp:
  servers:
    context7:
      name: context7
      command: npx
      args: ["-y", "@upstash/context7-mcp"]
      env: {}
      enabled: true
      built_in: true
      description: "Built-in doc search (Context7)"
    headroom:
      name: headroom
      command: headroom
      args: ["mcp", "serve"]
      env: {}
      enabled: true
      built_in: true
      description: "Built-in token saving (Headroom)"
routing:
  thresholds:
    small_prompt_tokens: 2000
    medium_prompt_tokens: 12000
  family_overrides: {}
  default_models: {}
ui:
  theme: system
  markdown: true
  syntax_highlighting: true
  vim_keys: false
  mode: auto
```

`default_model` is the persistent default selected with `cntx model default`. It is
used when no `--model` override applies and routing does not select a model.

`ui.mode` can be `auto`, `counsel`, `allow`, `request-permission`, or `file-only`.

`ui.markdown` documents the default terminal behavior: assistant responses and
interactive help are rendered as markdown, so `**bold**`, inline code, lists, and
code fences are formatted instead of printed raw.

`counsel` uses a bounded evaluator pass plus a selected worker model while keeping
Auto-style permissions.

Built-in MCP servers (`context7`, `headroom`) are always present. Editing them in
config customizes them; built-ins cannot be removed, only disabled.

## Custom Headers

Use repeated `--header` flags:

```bash
cntx endpoint --change gateway \
  --header X-Workspace=team-a \
  --header X-Trace=true
```

## Secrets

Prefer the runtime key store:

```bash
cntx api-key add --provider anthropic --value sk-ant-...
```

Or environment variables:

```bash
cntx endpoint --change work --api-key-env OPENAI_API_KEY
```

Inline `api_key` exists for portability, but it writes the key into config. The
runtime store is preferred because it is gitignored and never ships with the
published package. See [API keys](api-keys.md).
