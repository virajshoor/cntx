# API References

Cntx uses provider APIs for model discovery instead of hardcoded model lists.

## OpenAI

- List models: <https://platform.openai.com/docs/api-reference/models/list>
- Endpoint used by Cntx: `GET https://api.openai.com/v1/models`
- Chat streaming: OpenAI-compatible `POST /v1/chat/completions` with `stream: true`

## Anthropic

- List models: <https://docs.anthropic.com/en/api/models-list>
- Endpoint used by Cntx: `GET https://api.anthropic.com/v1/models`
- Messages streaming: `POST /v1/messages` with `stream: true`

## OpenAI-Compatible APIs

OpenAI-compatible providers should expose:

- `GET {base_url}/models`
- `POST {base_url}/chat/completions`

Use `--provider open-ai-compatible` and pass the provider's base URL.

## Ollama Local

- API reference: <https://docs.ollama.com/api>
- List local models: <https://docs.ollama.com/api/tags>
- Endpoint used by Cntx: `GET http://localhost:11434/api/tags`
- Chat streaming: `POST http://localhost:11434/api/chat`

## Ollama Cloud

- Cloud docs: <https://docs.ollama.com/cloud>
- Authentication docs: <https://docs.ollama.com/api/authentication>
- Pricing and plans: <https://ollama.com/pricing>
- Cloud-enabled model search: <https://ollama.com/search?c=cloud>
- Endpoint used by Cntx: `GET https://ollama.com/api/tags`
- Chat streaming: `POST https://ollama.com/api/chat`
