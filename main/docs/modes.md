# Modes

Cntx Code includes an extensible permission policy. Modes work together with the
[edit sandbox](sandbox.md): the sandbox confines writes to the project root, and
the mode decides when to ask for writes, shell, and network access.

## Auto

Allows low-risk reads and asks before writes, shell actions, or network tools.

## Counsel

Uses a token-efficient mix of models.

- Haiku-class models evaluate and classify the request.
- Sonnet-class models handle small changes.
- Opus-class models handle refactors.

The evaluator receives a bounded prompt preview, and the worker receives the optimized prompt plus a short evaluator note. Permission behavior matches Auto.

## Allow

Allows assistant actions without prompting.

## Request Permission

Asks before any tool or file operation.

## File Only

Allows file reads and writes but denies shell and network tools.

These policies are represented in code separately from the UI so future tools and plugins can add operation types without rewriting the CLI.
