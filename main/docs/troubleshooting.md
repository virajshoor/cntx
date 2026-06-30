# Troubleshooting

## No Primary Endpoint

Create and select an endpoint:

```bash
cntx endpoint --new --name work --provider anthropic --api-key-env ANTHROPIC_API_KEY
cntx endpoint --set-primary work
```

## Missing API Key

Add a key to the runtime store (preferred):

```bash
cntx api-key add --provider anthropic --value sk-ant-...
cntx api-key list
```

Or set the environment variable referenced by the endpoint:

```bash
export OPENAI_API_KEY=your_key
export OLLAMA_API_KEY=your_ollama_key
```

Check config:

```bash
cntx config show
```

## Ollama Cloud Pro Model Is Rejected

Check that:

- the endpoint provider is `ollama-cloud`
- `OLLAMA_API_KEY` is exported
- the key belongs to the Ollama account with Pro, Max, or Team access
- the model name is exactly what Ollama lists, such as `deepseek-v4-pro:cloud`

Refresh direct cloud models:

```bash
cntx --refresh-models
```

List direct cloud models manually:

```bash
curl https://ollama.com/api/tags \
  -H "Authorization: Bearer $OLLAMA_API_KEY"
```

## No Models In Auto Mode

Refresh models:

```bash
cntx --refresh-models
```

Or configure a default model:

```bash
cntx endpoint --change work --default-model gpt-5.5
```

## Ollama Local Cannot Connect

Start Ollama and verify:

```bash
curl http://localhost:11434/api/tags
```

## Direct Cloud vs Local Cloud Proxy

Use `ollama-cloud` for direct API calls to `https://ollama.com`.

Use `ollama-local` for the local daemon at `http://localhost:11434`, including after `ollama signin`.

## Diagnostics

```bash
cntx doctor
```
