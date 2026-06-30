# API Keys

Cntx Code is BYOK: bring your own keys. Keys never live in source. They are stored
in a gitignored runtime file inside your config directory (`secrets.yaml`), created
automatically on first boot with restrictive `0600` permissions. The file is never
part of the published crate or binary, so a fresh build on any machine starts clean
and sets itself up.

## Manage Keys

```bash
cntx api-key add    --provider anthropic        # prompts for the key
cntx api-key add    --provider anthropic --value sk-ant-...   # non-interactive
cntx api-key change --provider openai --value sk-...
cntx api-key delete --provider anthropic
cntx api-key list
```

`--provider` accepts any provider label, for example `openai`, `anthropic`,
`open-ai-compatible`, `ollama-cloud`, `ollama-local`, or the name of a custom
provider preset. Labels are matched case-insensitively.

`api-key list` prints masked keys (last four characters only) so you can confirm
which providers are configured without exposing secrets in your terminal history.

## Resolution Order

When Cntx Code sends a request for an endpoint, it resolves the key in this order:

1. The endpoint's inline `api_key` (set with `cntx endpoint --new --api-key ...`)
2. The environment variable named by the endpoint's `api_key_env`
3. The runtime secrets store, keyed by provider kind

This means `cntx api-key add --provider anthropic` is enough to make any Anthropic
endpoint work, even if the endpoint itself has no key configured.

## Where Keys Live

```bash
cntx config path      # shows the config directory
ls -l "$(cntx config path)/secrets.yaml"
```

The secrets file is excluded from the repository (`.gitignore`) and from the
published package (`Cargo.toml exclude`). There is intentionally no source-level
key file, so a key can never be compiled into the binary by accident.

## Backing Up And Moving Keys

The `secrets.yaml` file is plain YAML. To move to a new machine, copy it to the new
config directory and run `cntx api-key list` to verify. For team setups, prefer
`api_key_env` referencing a shared secret manager instead of copying keys.