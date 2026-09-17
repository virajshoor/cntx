# Modes

Cntx Code includes an extensible permission policy. Modes work together with the
[edit sandbox](sandbox.md): the sandbox confines writes to the project root, and
the mode decides when to ask for writes, shell, and network access.

Canonical names are `auto-approve`, `all-approve`, `manual-approve`, `counsel`,
and `file-only`. The legacy names `auto`, `allow`, and `request-permission`
remain accepted aliases, and existing configuration files keep loading. The
default mode is `auto-approve` in both interactive and one-shot use.

## Decision table

`Ask` means cntx pauses and shows a plain-language request ("Cntx wants to:
write file "notes.txt"") and continues only after `y`/`yes`. It never means
allow silently. Declining produces a tool error without side effects and is
never retried through a different tool.

| Operation | auto-approve | all-approve | manual-approve | file-only | counsel |
| --- | --- | --- | --- | --- | --- |
| Explicit read/glob/grep tool | Allow | Allow | Ask | Allow | Allow |
| In-root write/edit/apply | Ask | Allow | Ask | Allow | Ask |
| Shell command | Ask | Allow | Ask | Deny | Ask |
| Outside-root direct write | Deny | Deny | Deny | Deny | Deny |

Outside-root writes become eligible only with `--allow-write <root>` or the
explicit `--dangerously-disable-sandbox` flag; their approval decision then
follows the selected mode. `all-approve` never disables file containment.

## auto-approve (default)

Allows reads and asks before writes and shell commands.

## all-approve

Executes permitted tools without prompting. Direct file-write containment still
applies: writes outside allowed roots are denied regardless of mode.

## manual-approve

Asks before every tool, file, or shell operation. In this mode the implicit
repository scan and content reads are skipped; a single explicit approval
request covers any requested `@file`/project instruction/memory bundle before
its contents are read and sent.

## file-only

Allows file reads and writes but denies shell and network tools.

## counsel

Uses a token-efficient mix of models:

- Haiku-class models evaluate and classify the request.
- Sonnet-class models handle small changes.
- Opus-class models handle refactors.

The evaluator receives a bounded prompt preview, and the worker receives the
optimized prompt plus a short evaluator note. The worker runs through the same
tool/context/goal path as other action modes, even when evaluator and worker
use the same model. Approval behavior matches auto-approve.

## Selecting modes

```bash
cntx --mode all-approve "run the test suite and fix what fails"
cntx --mode manual-approve "show me exactly what you would change"
```

- `/mode` prints the current mode and the five available modes.
- `/mode <name>` validates first; invalid input changes nothing. Applies to
  this session and updates runtime and sandbox policy together.
- Shift+Tab cycles `auto-approve → all-approve → manual-approve →
  auto-approve` (legacy extra modes return to auto-approve) and preserves the
  current input draft.

## Tool execution

Every tool runs through the same order:

```text
validate name and arguments
→ resolve paths and check containment/exclusions
→ apply permission policy
→ block writes/commands if dry-run
→ show exact action and ask when required
→ execute once
→ capture bounded result and exit/error status
→ persist result
→ let the model take its next step
```

The sandbox is an application policy layer, **not OS isolation**: an approved
shell command can access the wider machine, like any command you type yourself.

These policies are represented in code separately from the UI so future tools
and plugins can add operation types without rewriting the CLI.
