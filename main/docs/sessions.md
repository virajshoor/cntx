# Sessions

Cntx stores sessions as YAML in the config directory.

```bash
cntx session list
cntx session resume          # latest session
cntx session resume <id>
cntx session export <id> session.json
cntx session import session.json
```

Interactive sessions are saved automatically after assistant responses.

The session format is intentionally plain YAML/JSON so future tools can index, search, compact, and migrate sessions.
