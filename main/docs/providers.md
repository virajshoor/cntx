# Providers

Cntx Code is BYOK: bring your own keys, endpoints, models, and provider accounts.
Provider setup is centered around endpoints. An endpoint is a named provider
configuration that can be selected directly or used as the primary endpoint for auto
mode.

For gateways that need custom base URLs, headers, or request paths, define a
[custom provider preset](custom-providers.md) and create an endpoint from it.

## Endpoint Fields

Each endpoint stores:

- `name`
- `provider`
- `api_key` or `api_key_env`
- `base_url`
- `default_model`
- `custom_headers`
- `timeout_secs`
- `metadata`
- `ollama_cloud` options, only for Ollama Cloud endpoints

Prefer `api_key_env` over inline `api_key` on shared machines, or use the runtime
key store with `cntx api-key add --provider <label>`. See [API keys](api-keys.md).

## OpenAI

```bash
export OPENAI_API_KEY=your_key

cntx endpoint --new \
  --name openai \
  --provider open-ai \
  --api-key-env OPENAI_API_KEY \
  --default-model gpt-5.5
```

## Anthropic

```bash
export ANTHROPIC_API_KEY=your_key

cntx endpoint --new \
  --name anthropic \
  --provider anthropic \
  --api-key-env ANTHROPIC_API_KEY \
  --default-model claude-sonnet-4.5
```

## OpenAI-Compatible APIs

Use this for gateways and providers that expose OpenAI-style `/models` and `/chat/completions` endpoints.

```bash
export GATEWAY_API_KEY=your_key

cntx endpoint --new \
  --name gateway \
  --provider open-ai-compatible \
  --base-url https://gateway.example.com/v1 \
  --api-key-env GATEWAY_API_KEY
```

## Ollama Local

Use this when the local Ollama daemon is serving requests.

```bash
cntx endpoint --new \
  --name local \
  --provider ollama-local \
  --base-url http://localhost:11434
```

If you are signed in with `ollama signin`, local Ollama can also proxy some cloud models:

```bash
cntx endpoint --new \
  --name ollama-signed-in \
  --provider ollama-local \
  --base-url http://localhost:11434 \
  --default-model deepseek-v4-pro:cloud
```

## Ollama Cloud

Use this for direct API access to `https://ollama.com`.

```bash
export OLLAMA_API_KEY=your_key

cntx endpoint --new \
  --name ollama-cloud \
  --provider ollama-cloud
```

When `--provider ollama-cloud` is used and no key is specified, Cntx automatically uses `OLLAMA_API_KEY`.

## Ollama Cloud Pro, Max, And Subscription Models

Ollama Pro and Max do not require a separate API host. Direct cloud API access uses `https://ollama.com`; your API key and account plan decide which cloud models are allowed.

```bash
cntx endpoint --new \
  --name ollama-pro \
  --provider ollama-cloud \
  --ollama-cloud-plan pro \
  --default-model deepseek-v4-pro:cloud
```

Supported plan labels:

- `free`
- `pro`
- `max`
- `team`

For `pro`, `max`, and `team`, Cntx records `subscription_models: true` in the endpoint config. This is metadata for routing and clarity; Ollama still enforces actual access from the API key.

Example model names:

```bash
cntx --endpoint ollama-pro --model deepseek-v4-pro:cloud "review the architecture"
cntx --endpoint ollama-pro --model gpt-oss:120b-cloud "summarize this module"
```

More detail: [Ollama Cloud and Pro](ollama-cloud.md).

## Set Primary Endpoint

```bash
cntx endpoint --set-primary ollama-pro
```

The primary endpoint is used when you do not pass `--endpoint`.

## Refresh Models

```bash
cntx --refresh-models
```

Refresh fetches provider model lists, updates the model cache, preserves aliases, and marks missing models as deprecated.

## YAML Import

```yaml
primary_endpoint: ollama-pro
endpoints:
  - name: ollama-pro
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
```

```bash
cntx endpoint --import providers.yaml
```
