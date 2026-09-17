# Team Admin Guide

How to roll out cntx to staff with shared endpoints, controlled keys, and
repeatable updates.

## Standardize with one preset file

Define endpoints once in YAML (no keys in the file) and distribute it:

```yaml
endpoints:
  - name: work
    provider: anthropic
    api_key_env: ANTHROPIC_API_KEY
```

Each machine imports it:

```bash
cntx endpoint --import providers.yaml
cntx endpoint --set-primary work
```

Custom providers work the same way: `cntx provider install-preset <name>`
from a shared [custom preset](custom-providers.md).

## Key distribution and revocation

- Preferred: each user runs `cntx api-key add --provider <name>` so keys live
  in their own `secrets.yaml` (mode `0600`, never in source).
- Alternative: set the env var the endpoint references (e.g.
  `ANTHROPIC_API_KEY`) via your existing device management.
- Revocation: delete the key at the provider, then
  `cntx api-key delete --provider <name>` on the machine, or unset the env
  var. There is no central key server; treat workstation keys like SSH keys.
- Back up or move a workstation: copy the config dir
  (`~/Library/Application Support/cntxcode/` on macOS,
  `~/.config/cntxcode/` on Linux) or re-run `init`. Override the location
  per-machine with `CNTX_CONFIG_DIR`.

## Updating the fleet

```bash
cargo install cntx --force
```

Pin versions by installing from a tag checkout when you need everyone on the
same release. `cntx doctor` verifies config, keys, and model cache per
machine; `cntx doctor --fix` repairs missing files.

## Sensible team defaults

- Start everyone on `auto-approve` (in-project writes run, commands ask once
  per session with `ya`). Power users can opt into `all-approve`; auditors
  can use `manual-approve` or `file-only`.
- Point project memory at `.cntx/memory.md` and shared instructions at
  `AGENTS.md` so team conventions load automatically.
- Sessions persist locally per machine; export with `cntx session --export`
  when handoff is needed.

## What cntx does not do (yet)

No central admin console, no SSO, no remote audit log, no vault/KMS
integration. See [Enterprise Readiness](enterprise-readiness.md) for the
status matrix and workarounds.
