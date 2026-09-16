# Sessions

Cntx stores sessions as YAML in the config directory.

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
transition, so an interrupted turn keeps its history. Each session records:

- the workspace root it belongs to,
- an optional compaction summary and the index of the first message it covers,
- the current goal (objective, status, progress, evidence, step counts).

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

The session format is intentionally plain YAML/JSON so future tools can index,
search, compact, and migrate sessions.
