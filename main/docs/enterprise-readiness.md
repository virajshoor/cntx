# Enterprise Readiness

Status of enterprise controls in cntx. "Today" describes the current release;
"Roadmap" is planned, not promised.

## Status matrix

| Control | Today | Workaround / Roadmap |
| --- | --- | --- |
| SSO / SAML / OIDC / SCIM | Not supported; identity is local user | Roadmap: external IdP for team console |
| Central audit log | Local YAML session transcripts only | Roadmap: append-only export (SIEM/S3); today: collect `sessions/` via MDM |
| RBAC / team roles | Five global approval modes per user | Enforce `manual-approve`/`file-only` by policy; roadmap: managed policy push |
| Secrets management | Local `secrets.yaml` (0600) + env vars | Distribute via existing device management; roadmap: vault/KMS |
| Key rotation / expiry | Manual at provider + re-add | Document rotation runbook per provider |
| Deployment (MDM/GPO/containers) | `cargo install` per machine | Script install + `endpoint --import`; roadmap: signed binaries, containers |
| Data residency | BYOK: data goes to your provider | Choose provider regions; self-hosted Ollama Local keeps data on-machine |
| DLP / PII redaction | Secret-filename blocklist only | Review prompts before sending; roadmap: redaction filters |
| Audit of shell actions | Transcript shows commands run | Collect transcripts centrally today |
| SLA / support tier | Best effort via repo issues | Paid support: roadmap |

## Recommended posture today

1. Standardize one preset file and distribute via `endpoint --import`
   ([Team Admin Guide](team-admin-guide.md)).
2. Default users to `auto-approve`; restrict sensitive repos to
   `manual-approve` or `file-only`.
3. Forbid `--dangerously-disable-sandbox` by policy.
4. Prefer Ollama Local or a region-pinned provider for regulated data.
5. Collect `sessions/` and config dirs for audit via existing endpoint
   management.

## Compliance notes

SOC 2 / GDPR / HIPAA mappings do not exist yet because the centralized
logging, retention, and deletion controls they require are on the roadmap.
The [Security Overview](security-overview.md) documents current data flow
for your own assessment.
