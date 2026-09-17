# Sessions

Cntx persists conversation state locally as YAML under the configuration
directory (`$CNTX_CONFIG_DIR/sessions/`; on macOS
`~/Library/Application Support/cntxcode/sessions/`, on Linux
`~/.config/cntxcode/sessions/`). Session ids become filenames; unsafe ids
(path separators, `..`, leading `.`/`-`, over 128 characters) are rejected
and never reach the filesystem.

```bash
cntx session list
cntx session resume          # latest session for this workspace, interactive
cntx session resume <id>
cntx session export <id> session.json
cntx session import session.json
```

## What is persisted

Sessions save user messages, assistant tool requests, tool results, and final
responses. Saving happens atomically after each tool result and state
transition — the file is written to a temporary name and renamed, so an
interrupted turn or a crash keeps its history. Each session records:

- the workspace root it belongs to,
- an optional compaction summary and the index of the first message it covers
  (older messages stay on disk but are omitted from requests),
- the current goal (objective, status, progress, evidence, step counts).

## Importing and exporting

Exports are pretty-printed JSON; imports accept either format. Imported data
is untrusted: a missing or unsafe session id is replaced with a fresh one
(warned on stderr), and an import whose id already exists on disk never
overwrites the existing session — it gets a new id too. The format is
intentionally plain YAML/JSON so future tools can index, search, compact,
and migrate sessions.

## Resuming

- `/resume <session-id>` loads a saved session and continues interactively.
- `/resume` loads the latest session for the current workspace.
- `cntx session resume [id]` enters the interactive loop with that session
  instead of printing YAML and exiting.

Starting or loading a session never silently changes workspace roots: a session
from another directory shows its path and expects you to launch Cntx there.
Legacy sessions without a workspace root use the current workspace with a
visible notice.

## Compaction

`/compact` summarizes covered older messages through the resolved provider
while preserving the latest two user turns and their tool results. The session
id stays stable, the summary and covered index are stored, and the full
transcript remains on disk. The same mechanism compacts automatically when a
request would exceed the context budget; the summary, goal, decisions, and
changed paths survive. If compaction fails, you get a clear error and the
session is preserved — requests are never sent oversized.

## Follow-up context

New turns can reference earlier file changes and command results because tool
results stay in the conversation history, bounded by `routing.history_turns`
and the context budget.
