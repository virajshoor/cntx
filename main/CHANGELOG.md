# Changelog

## 0.3.3 - 2026-07-13

- Fix sandbox blocking writes and shell in interactive mode: the sandbox's
  internal PermissionPolicy was still using Auto mode even when the runtime
  mode was overridden to Allow. Now the sandbox is built with the correct
  effective mode so the model can actually write files and run commands.
- Replace the "thinking..." dot animation with a live preview of the last
  ~80 characters of streamed output, like Claude Code. You can see the
  model's response as it generates instead of just dots.

## 0.3.2 - 2026-07-13

- Fix sandbox blocking in interactive mode: default to allow mode so the model
  can actually write files and run shell commands without being blocked.
- Fix Shift+Tab: now cycles through permission modes (Auto -> Allow -> Counsel
  -> FileOnly -> RequestPermission -> Auto) like Claude Code, instead of
  indent/dedent which didn't work in emacs mode.
- Remove +tools from the prompt bar (tool-use is always on in interactive mode).

## 0.3.1 - 2026-07-13

- Enable tool-use by default in interactive mode so the model can read, write,
  edit files, and run shell commands without needing `--tool-use`.
- Add `/tools` slash command to toggle tool-use on/off in the interactive shell.
- Add base system prompt telling the model it runs locally on the user's
  machine (not a browser chat), can access the filesystem, and should respond
  in the user's language.
- Fix context scan hanging on non-project directories: `project_root()` now
  searches upward for a `.git` or `.cntx` marker; the recursive context scan
  is skipped entirely when no marker is found (e.g. home directory).
- Bound the recursive context scan with a max depth (10) and max file count
  (5000) to prevent hangs on very large projects.
- Expand the directory exclusion list for the context scan (`.cargo`, `.npm`,
  `Library`, `.config`, `.venv`, `__pycache__`, etc.).
- Fix Shift+Tab: now properly indents/dedents using `indent_size=4` and emacs
  mode. Tab now indents the current line.
- Show `+tools` in the interactive prompt bar when tool-use is active.

## 0.3.0 - 2026-07-13

- Load prior session turns into each new prompt so multi-turn conversation
  context is preserved in the interactive shell. Bounded by
  `config.routing.history_turns` (default 10).
- Persist interactive shell command history across restarts via a
  `history.txt` file in the config directory.
- Activate skills: `/skill <name>` injects the skill's prompt as a system
  message so reusable instructions shape model behavior for the session.
- Harden the edit sandbox against symlink escapes. Reject writes through
  symlinks that resolve outside the project root, with regression tests.
- Add a 60-second timeout to tool-use shell commands. Hanging commands are
  killed instead of freezing the CLI.
- Exclude secret-named files (`.env`, `secrets.yaml`, `id_rsa`, etc.) from
  grep and glob results to prevent credential leaks.
- Pin the built-in Context7 MCP server to `@upstash/context7-mcp@3.2.3`
  instead of fetching the latest version at runtime.
- Add `doctor --verify` to run `cargo fmt`, `cargo clippy`, `cargo test`, and
  `cargo build` and report pass/fail.
- Validate that `cntx bench --endpoint` refers to an existing endpoint
  instead of silently falling back to `<auto>`.
- Generate a new session ID on import when the ID already exists, instead of
  silently overwriting the existing session.
- Extract shared apply logic into `Runtime::apply_files` to remove duplication
  between normal and counsel prompt flows.
- Unify the secret-file blocklist into a single `blocklist` module used by
  context selection, the optimizer, and the tool-use loop.
- Fix clippy and rustfmt violations that were failing the verification gate.
- Add a GitHub Actions CI workflow that runs fmt, clippy, test, and build.
- Clarify in MCP docs that the autonomous agent loop is not yet implemented.

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
