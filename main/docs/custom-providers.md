# Custom Providers

Custom providers are reusable presets that describe how to configure an existing
adapter family for a specific gateway. They are not new adapter implementations;
they tell Cntx Code how to reuse the OpenAI-compatible, Anthropic, or Ollama
adapter for a particular endpoint.

## Built-In Gallery

```bash
cntx provider gallery
cntx provider install-preset openrouter
cntx provider use openrouter
```

The gallery currently includes OpenRouter, Groq, Together, and Fireworks presets.
Presets configure the base URL, API key environment variable, adapter family, and a
reasonable default model. You can edit the generated config later with
`cntx config show` and `cntx provider add`.

## Add A Preset From Flags

```bash
cntx provider add \
  --name gateway \
  --kind open-ai-compatible \
  --base-url https://gateway.example.com/v1 \
  --api-key-env GATEWAY_API_KEY \
  --default-model gpt-4o \
  --header X-Workspace=team-a
```

`--kind` accepts `open-ai-compatible`, `anthropic`, or `ollama`.

## Add Presets From YAML

```yaml
# providers.yaml
providers:
  - name: gateway
    kind: open-ai-compatible
    base_url: https://gateway.example.com/v1
    api_key_env: GATEWAY_API_KEY
    default_model: gpt-4o
    headers:
      X-Workspace: team-a
    models_path: v1/models
    chat_path: v1/chat/completions
  - name: internal-llama
    kind: ollama
    base_url: http://10.0.0.5:11434
    default_model: llama3.2
```

```bash
cntx provider add --file providers.yaml
```

`models_path` and `chat_path` are optional overrides for the model-listing and chat
endpoints. When omitted, the adapter defaults are used (`models`, `chat/completions`
for OpenAI-compatible; `models`, `messages` for Anthropic; `api/tags`, `api/chat`
for Ollama).

## List And Remove

```bash
cntx provider list
cntx provider remove gateway
```

## Create An Endpoint From A Preset

```bash
cntx provider use gateway          # creates endpoint `gateway`, sets it primary
```

Or create with a different name and apply overrides:

```bash
cntx endpoint --new --name gw --from-preset gateway --default-model gpt-4o-mini
```

Then add a key and refresh models:

```bash
cntx api-key add --provider openai --value sk-...   # if the gateway uses OpenAI auth
cntx endpoint --set-primary gw
cntx --refresh-models
```

## How Custom Paths Work

When a preset sets `models_path` or `chat_path`, Cntx Code stores them in the
endpoint's `metadata` and the adapter reads them at request time. This lets a
gateway that uses non-standard paths work without a new adapter.
