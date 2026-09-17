# Skills

Skills are reusable instructions — project knowledge, coding standards, or
workflows — injected into every request as a system message until you
deactivate them. They let a team encode "how we do things" once and have the
assistant follow it in every session.

## Where skills live

Cntx reads skills from two YAML directories:

| Location | Purpose |
| --- | --- |
| `<config>/skills/` | Personal skills, available in every project (`cntx config path` shows the folder; `skills/` sits beside it) |
| `.cntx/skills/` | Project skills, committed with the repository so teammates load them automatically |

Skills from both directories are merged and sorted alphabetically by name.
Each file is one YAML document with extension `.yaml`; invalid files are
reported with their path rather than skipped silently.

## Skill file format

A skill is a plain YAML document with four fields:

```yaml
name: repo-standards
description: Apply repository coding and testing standards
created_at: 2026-09-17T00:00:00Z
prompt: |
  Follow these standards when editing code:
  - ... your actual instructions ...
```

- `name` — how you reference the skill (`/skill repo-standards`). Must be
  unique; the first match on lookup wins.
- `description` — shown in `cntx skill list` (one-line summary).
- `created_at` — set automatically for created skills; informational only.
- `prompt` — the instruction text injected as a system message. This is the
  part the model reads; write it like you would write onboarding notes.

## Creating and managing from the shell

```bash
cntx skill new repo-standards "Apply repository coding and testing standards"
cntx skill list
cntx skill show repo-standards
```

- `cntx skill new` writes a starter file into `<config>/skills/<name>.yaml`
  whose `prompt` is a placeholder you edit:
  `Use this skill when the task matches: <description>` plus an
  "Add reusable instructions here" marker.
- `cntx skill show <name>` prints the full YAML, so you can edit it, or copy
  it into `.cntx/skills/` to ship it with a repository.
- There is no `delete` subcommand yet; remove the YAML file directly. File
  names must end in `.yaml` (files with other extensions are ignored).

## Activating in a session

```text
/skills            list available skills
/skill <name>      activate a skill for this session
/skill             show the active skill (or a hint if none)
```

While a skill is active, its `prompt` is sent as an additional system message
with **every** request — in the interactive prompt loop, one-shot prompts,
apply mode, and goal runs. One skill is active at a time; activating another
replaces it.

The starter skill created by `cntx skill new` has only placeholder
instructions, so replace its `prompt` with real content before relying on it.
