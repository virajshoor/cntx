# Changelog

## 0.2.1 - 2026-07-12

- Fix Shift+Tab to dedent (remove indentation) instead of inserting a tab character.

## 0.2.0 - 2026-07-12

- Add `--tool-use` mode: the model can call tools (read, write, edit, bash,
  glob, grep) in a multi-turn loop, similar to Claude Code's agent loop.
- Add thinking indicator: shows "thinking..." with animated dots while the
  model generates, clearing when the first token arrives.
- Add Shift+Tab keybinding in the interactive shell for inserting tab characters.
- Add `/theme` slash command to toggle between dark and light terminal themes.
- Add `UiConfig.theme` field to persist the chosen theme in config.
- Improve markdown rendering with theme-aware colors.

## 0.1.3 - 2026-06-30

- Add `cntx --docs`, an interactive packaged docs browser.
- Include `EXPLAIN.md` in the published crate and docs browser.
- Add `cntx init` for first-run endpoint setup.
- Add `cntx doctor --fix` for safe local config repairs.
- Add `cntx bench` for token, routing, and rough cost previews.
- Add `cntx demo` for a no-key local product demo.
- Add `cntx completions <shell>` for shell completion generation.
- Add `cntx memory` for project-local notes in `.cntx/memory.md`.
- Add built-in provider preset gallery and install command.
- Allow `cntx session resume` to resume the latest session when no id is passed.

## 0.1.2 - 2026-06-30

- Include public README, docs, changelog, and license in the published crate.
- Clarify install and first-run guidance for crates.io users.
- Prepare the Homebrew formula for the new patch release.

## 0.1.1 - 2026-06-30

- Add sandboxed apply mode for `path=`, `file=`, and `filename=` fenced blocks.
- Keep the last apply checklist available in interactive chat.
- Improve terminal markdown rendering and add a working-dot animation.
- Add the static website.
