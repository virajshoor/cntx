# Sandbox

Cntx Code confines the assistant's file edits to the project root by default. This is
a core safety feature: an uncontrolled or rogue model run cannot rewrite files
outside the workspace or touch the wider machine.

## What The Sandbox Does

- File writes are allowed only inside the project root (or an explicitly allowed
  directory). Writes anywhere else are denied, even in `allow` mode.
- Shell and network access are gated by the active permission mode.
- Reads are always permitted by the sandbox; the mode decides separately.

The sandbox is a policy layer consulted before file writes. It is enabled by
default on every run and is used by apply mode today.

## Apply Mode

Apply mode turns model-proposed files into real writes:

```bash
cntx --apply --mode allow "add a focused test helper"
```

Cntx asks the model to emit complete fenced code blocks annotated with `path=`,
`file=`, or `filename=`, then writes those blocks through the sandbox. Quoted paths
are accepted, and paths outside the project or allowed roots are denied. After the
run, Cntx prints a file checklist with each written, blocked, or outside-sandbox
path.

In the interactive shell:

```text
/apply
/checklist
```

`/checklist` shows the last apply result until the next apply run.

## Check The Active Sandbox

```bash
cntx sandbox
cntx sandbox --yaml
```

In the interactive shell:

```text
/sandbox
```

## Extend The Sandbox

Allow an additional writable directory with `--allow-write` (repeatable):

```bash
cntx --allow-write /Users/you/shared-libs "explain the shared utilities"
```

## Disable The Sandbox

```bash
cntx --dangerously-disable-sandbox "edit anywhere"
```

This removes path containment entirely. Use it only when you understand the risk;
it exists for workflows where the assistant must edit across many roots.

## Modes And The Sandbox

The sandbox works together with permission modes:

- `auto-approve` (default): reads and in-project writes allowed; shell and
  network ask first; commands can be approved for the session with `ya`.
- `all-approve`: writes inside the sandbox proceed without asking; shell and
  network proceed without asking. Writes outside the sandbox are still denied.
- `manual-approve`: ask before any operation.
- `counsel`: token-efficient model mix; permission behavior matches
  `auto-approve`.
- `file-only`: file reads and writes allowed (within the sandbox); shell and
  network denied.

Legacy names `auto`, `allow`, and `request-permission` still work as aliases.

The sandbox is an application policy layer, not OS isolation: an approved shell
command can access the wider machine. Path containment covers direct file
writes (including symlink and traversal escape attempts).

See [Modes](modes.md) for the full mode reference.
