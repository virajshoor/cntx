# Auto Routing

Auto routing uses the primary endpoint unless you pass `--endpoint`.

Cntx does not infer task complexity. Routing is based primarily on prompt length after optimization:

1. Cntx normalizes prompt whitespace.
2. It removes duplicate natural-language context while preserving code fences.
3. It estimates token count.
4. It maps the prompt to `small`, `medium`, or `large`.
5. It selects a matching model family from the refreshed model cache.

## Single Available Model

If the refreshed cache has exactly one available model for the endpoint, auto mode
always uses it - regardless of prompt size or the configured default. This keeps a
fresh setup with a single model working without falling through to the size-based
fallback or erroring. Once a second model appears, normal size-based routing
resumes.

Default thresholds:

```yaml
routing:
  thresholds:
    small_prompt_tokens: 2000
    medium_prompt_tokens: 12000
```

## Family Discovery

Cntx avoids hardcoded full model lists. It discovers model families from provider responses and model names:

- Anthropic: Haiku, Sonnet, Opus
- OpenAI-compatible: nano, mini, small, pro, large, max
- Ollama Local: parameter sizes such as `8B`, `32B`, `70B`
- Ollama Cloud: usage metadata, cloud/pro names, and sizes such as `120B` or `1.6T`

## Ollama Cloud Pro And Max

Cloud subscription model names such as `deepseek-v4-pro:cloud` are treated as large-route candidates. Names such as `gpt-oss:20b-cloud` are treated as smaller cloud candidates than `gpt-oss:120b-cloud`.

Actual access is still enforced by Ollama through the API key attached to your account.

## Counsel Mode

Counsel mode adds a token-efficient multi-model pass:

1. A Haiku-class or small model receives a bounded evaluation prompt.
2. Cntx classifies the request as evaluation, small change, or refactor.
3. Evaluation tasks can finish with the evaluator.
4. Small changes go to a Sonnet-class or medium model.
5. Refactors go to an Opus-class or large model.

This mode avoids sending the full optimized prompt to every model. The evaluator receives a capped preview, while the worker receives the optimized prompt and a short evaluator note.

## Overrides

You can override route mappings per endpoint:

```yaml
routing:
  family_overrides:
    ollama-pro:
      small: gpt-oss:20b-cloud
      medium: gpt-oss:120b-cloud
      large: deepseek-v4-pro:cloud
```

Endpoint `default_model` is used when configured and available.
