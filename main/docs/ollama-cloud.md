# Ollama Cloud And Pro

Cntx supports direct Ollama Cloud access through the `ollama-cloud` provider.

The important detail: Ollama Cloud Free, Pro, Max, and Team all use the same direct API host, `https://ollama.com`. The API key attached to your Ollama account controls which cloud and subscription models are available.

## Direct API Setup

Create an Ollama API key in your Ollama account and export it:

```bash
export OLLAMA_API_KEY=your_api_key
```

Create a Cntx endpoint:

```bash
cntx endpoint --new \
  --name ollama-pro \
  --provider ollama-cloud \
  --ollama-cloud-plan pro \
  --default-model deepseek-v4-pro:cloud
```

Set it as primary and refresh models:

```bash
cntx endpoint --set-primary ollama-pro
cntx --refresh-models
```

`--provider ollama-cloud` defaults to `OLLAMA_API_KEY` when no explicit key or key environment variable is provided.

## Plans

Plan labels are stored in config for clarity:

```bash
cntx endpoint --change ollama-pro --ollama-cloud-plan max
```

Supported labels:

- `free`
- `pro`
- `max`
- `team`

For `pro`, `max`, and `team`, Cntx marks the endpoint as intended for subscription models. Ollama still decides access from the API key.

## Model Names

Use the exact model names exposed by Ollama. Cntx does not rewrite names.

Examples:

```bash
cntx --model deepseek-v4-pro:cloud "review this repository"
cntx --model gpt-oss:120b-cloud "summarize the architecture"
```

You can list direct cloud models with:

```bash
curl https://ollama.com/api/tags \
  -H "Authorization: Bearer $OLLAMA_API_KEY"
```

Cntx uses the same endpoint for `cntx --refresh-models`.

## Auto Routing With Cloud Models

Cntx routes by optimized prompt size:

- small prompts prefer small/light cloud models
- medium prompts prefer medium models
- large prompts prefer Pro/Max-style heavy models, trillion-scale models, or models with high usage metadata

Examples of large-route model names:

- `deepseek-v4-pro:cloud`
- `gpt-oss:120b-cloud`
- models reporting parameter sizes such as `1.6T`

You can always override routing with `--model`.

## Local Signed-In Proxy

Ollama can also run cloud models through the local Ollama service after:

```bash
ollama signin
```

In that case, configure Cntx as `ollama-local`:

```bash
cntx endpoint --new \
  --name ollama-signed-in \
  --provider ollama-local \
  --base-url http://localhost:11434 \
  --default-model deepseek-v4-pro:cloud
```

Use `ollama-cloud` for direct API access. Use `ollama-local` for local daemon access.
