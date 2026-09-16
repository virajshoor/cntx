# OpenCode Go

[OpenCode Go](https://opencode.ai/docs/go/) is a $10/month subscription that
provides access to popular open coding models. Cntx Code ships it as a built-in
provider preset.

## Setup

```bash
cntx api-key add --provider opencode-go     # paste your Go key once
cntx provider install-preset opencode-go
cntx provider use opencode-go
cntx --refresh-models
cntx
```

- The preset points at the official base URL `https://opencode.ai/zen/go/v1`.
- The key resolves from `OPENCODE_GO_API_KEY` (environment) or the runtime key
  store label `opencode-go`. A Go endpoint never falls back to an unrelated
  provider's key.
- Models are fetched dynamically from the `/models` endpoint; the default model
  is `glm-5.3-flash`, confirmed in the fetched list. Use `/models` to see the
  full current catalog and `/model <id>` to switch.
- Display names like `opencode-go/<model-id>` are normalized to the bare id for
  Go endpoints only.

## Client identity and session headers

Per the official docs, Go requires a client-specific user agent and a stable
conversation session id:

- Every Go request sends `User-Agent: cntx/<package version>`.
- Every Go request in one conversation sends the same
  `x-opencode-session: <session id>` — across tool turns, counsel requests,
  retries, and compaction. `/clear` starts a new session id.

## Protocol routing

Go models use different API paths, so Cntx routes by model family instead of
forcing one preset:

| Model families | API path |
| --- | --- |
| GLM, Kimi, LongCat, DeepSeek, MiMo, Hy | `/chat/completions` |
| MiniMax, Qwen | `/messages` (Anthropic-compatible) |
| GPT 5.6 Luna, Grok 4.6, Muse Spark Contributor | `/responses` (OpenAI Responses) |

Because the fetched model list contains ids without protocol metadata, the
routing uses a family table plus an explicit endpoint protocol override
(`endpoint metadata: protocol: chat|messages|responses`). Unclassified families
report that an override is required rather than guessing. All system messages
are merged for the Anthropic-compatible path so skills, summaries, and goal
instructions are never dropped.

## Errors and limits

- 401/403 (bad key), 429 (rate limit), unsupported model, and server errors are
  surfaced clearly; authentication errors are never retried automatically.
- A broken stream is not retried after content was already shown, so partial
  output is never duplicated.
- Usage limits, prices, and the model catalog change over time; the official
  docs are the source of truth. Cntx does not promise unlimited usage.

## Verification status

Protocol behavior (URL paths, auth header, user agent, session header, and
streamed text parsing for all three protocols) is verified with local HTTP
mocks in `main/tests/agent_flow.rs`. No authenticated live subscription test
was run; usage against real Go endpoints has not been exercised in this
release.
