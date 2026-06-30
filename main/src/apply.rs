//! File application.
//!
//! In `--apply` mode, Cntx Code asks the model to emit any file it wants to
//! create or change as a fenced code block annotated with `path=<relative
//! path>`. After the response is generated, these blocks are extracted and
//! written through the edit sandbox so writes stay confined to the project
//! root. A checklist of written and blocked files is printed.

use std::path::{Path, PathBuf};

use owo_colors::OwoColorize;

use crate::permissions::PermissionDecision;
use crate::sandbox::Sandbox;

const PATH_KEYS: [&str; 3] = ["path", "file", "filename"];

/// Instruction prepended to the prompt in `--apply` mode so the model emits
/// files as path-annotated fenced blocks that Cntx Code can write to disk.
pub const APPLY_SYSTEM_INSTRUCTION: &str = "\
You are running in apply mode. When you create or modify a file, output its full \
contents as a fenced code block annotated with the target path, like:\n\
```rust path=src/main.rs\n<full file contents>\n```\n\
Rules:\n\
- Use a relative path from the project root. Never use absolute paths.\n\
- Output the complete file contents, not a diff or a fragment.\n\
- One file per fenced block. Annotate every block you want written with `path=`.\n\
- Fenced blocks without a `path=` are shown but not written.\n\
- After the blocks, briefly summarize what each file is for.\n\
- Stay inside the project; do not propose files outside it.";

/// A file the model proposed, parsed from a fenced code block.
#[derive(Clone, Debug)]
pub struct ProposedFile {
    pub path: PathBuf,
    pub language: String,
    pub content: String,
}

/// Result of attempting to write one proposed file.
#[derive(Clone, Debug)]
pub struct ApplyOutcome {
    pub path: PathBuf,
    pub status: ApplyStatus,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyStatus {
    Written,
    Blocked,
    OutsideSandbox,
    Error,
}

impl ApplyStatus {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Written => "[written]",
            Self::Blocked => "[blocked]",
            Self::OutsideSandbox => "[outside sandbox]",
            Self::Error => "[error]",
        }
    }
}

/// Extract proposed files from model output. Only fenced blocks that carry a
/// `path=` (or `file=` / `filename=`) annotation are treated as files; other
/// blocks are left as display-only markdown.
pub fn extract_files(text: &str) -> Vec<ProposedFile> {
    let mut files = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("```") {
            continue;
        }
        let info = trimmed.trim_start_matches("```").trim();
        let Some((path, language)) = parse_path_from_info(info) else {
            continue;
        };
        let mut content = String::new();
        for body in lines.by_ref() {
            if body.trim_start().starts_with("```") {
                files.push(ProposedFile {
                    path: PathBuf::from(path),
                    language: language.to_string(),
                    content,
                });
                break;
            }
            content.push_str(body);
            content.push('\n');
        }
    }
    files
}

fn parse_path_from_info(info: &str) -> Option<(String, String)> {
    let tokens = split_info_tokens(info);
    let language = tokens
        .iter()
        .find(|part| !part.contains('='))
        .cloned()
        .unwrap_or_default();

    for token in tokens {
        if let Some((key, value)) = token.split_once('=') {
            if PATH_KEYS
                .iter()
                .any(|accepted| key.eq_ignore_ascii_case(accepted))
            {
                let path = value.trim_matches(|c| c == '"' || c == '\'').to_string();
                return Some((path, language));
            }
        }
    }
    None
}

fn split_info_tokens(info: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;

    for ch in info.chars() {
        match (quote, ch) {
            (Some(active), value) if value == active => {
                current.push(value);
                quote = None;
            }
            (None, '"' | '\'') => {
                current.push(ch);
                quote = Some(ch);
            }
            (None, value) if value.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

/// Apply a list of proposed files through the sandbox. Paths are resolved
/// relative to `root` and must remain inside an allowed write root.
pub fn apply(sandbox: &Sandbox, files: &[ProposedFile], root: &Path) -> Vec<ApplyOutcome> {
    files
        .iter()
        .map(|file| write_one(sandbox, file, root))
        .collect()
}

fn write_one(sandbox: &Sandbox, file: &ProposedFile, root: &Path) -> ApplyOutcome {
    let target = if file.path.is_absolute() {
        file.path.clone()
    } else {
        root.join(&file.path)
    };

    let verdict = sandbox.evaluate(crate::permissions::Operation::WriteFile, Some(&target));
    match verdict.decision {
        PermissionDecision::Allow => {
            if let Some(parent) = target.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    return ApplyOutcome {
                        path: file.path.clone(),
                        status: ApplyStatus::Error,
                        reason: format!("could not create directory: {error}"),
                    };
                }
            }
            match std::fs::write(&target, &file.content) {
                Ok(()) => ApplyOutcome {
                    path: file.path.clone(),
                    status: ApplyStatus::Written,
                    reason: verdict.reason,
                },
                Err(error) => ApplyOutcome {
                    path: file.path.clone(),
                    status: ApplyStatus::Error,
                    reason: error.to_string(),
                },
            }
        }
        PermissionDecision::Deny => {
            let outside = !sandbox.is_within_allowed(&target);
            ApplyOutcome {
                path: file.path.clone(),
                status: if outside {
                    ApplyStatus::OutsideSandbox
                } else {
                    ApplyStatus::Blocked
                },
                reason: verdict.reason,
            }
        }
        PermissionDecision::Ask => ApplyOutcome {
            path: file.path.clone(),
            status: ApplyStatus::Blocked,
            reason: "interactive approval required; rerun in interactive mode or use --mode allow"
                .to_string(),
        },
    }
}

/// Print a checklist of apply outcomes.
pub fn print_checklist(outcomes: &[ApplyOutcome]) {
    if outcomes.is_empty() {
        return;
    }
    println!("{}", "file checklist".bold());
    for outcome in outcomes {
        let symbol = match outcome.status {
            ApplyStatus::Written => outcome.status.symbol().green().to_string(),
            ApplyStatus::OutsideSandbox => outcome.status.symbol().red().to_string(),
            _ => outcome.status.symbol().yellow().to_string(),
        };
        println!("  {} {} {}", symbol, outcome.path.display(), outcome.reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_path_annotated_blocks() {
        let text = "Here is the file:\n\n```rust path=src/main.rs\nfn main() {}\n```\n\nAnd a second:\n```python file=scripts/run.py\nprint(1)\n```\n";
        let files = extract_files(text);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, PathBuf::from("src/main.rs"));
        assert_eq!(files[0].language, "rust");
        assert_eq!(files[0].content, "fn main() {}\n");
        assert_eq!(files[1].path, PathBuf::from("scripts/run.py"));
    }

    #[test]
    fn extracts_quoted_path_annotations() {
        let text = "```toml path=\"fixtures/demo config.toml\"\nname = \"cntx\"\n```\n";
        let files = extract_files(text);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("fixtures/demo config.toml"));
        assert_eq!(files[0].language, "toml");
    }

    #[test]
    fn ignores_blocks_without_path() {
        let text = "```rust\nfn main() {}\n```\n";
        assert!(extract_files(text).is_empty());
    }

    #[test]
    fn apply_writes_file_inside_sandbox() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        let root = temp.path().canonicalize().unwrap();
        let sandbox = Sandbox::new(crate::permissions::Mode::Allow, root.clone(), Vec::new());
        let files = vec![ProposedFile {
            path: PathBuf::from("src/lib.rs"),
            language: "rust".to_string(),
            content: "pub fn x() -> i32 { 1 }\n".to_string(),
        }];
        let outcomes = apply(&sandbox, &files, &root);
        assert_eq!(outcomes[0].status, ApplyStatus::Written);
        assert!(root.join("src/lib.rs").exists());
    }

    #[test]
    fn apply_blocks_files_outside_sandbox() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        let root = temp.path().canonicalize().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_path = outside.path().canonicalize().unwrap();

        let sandbox = Sandbox::new(crate::permissions::Mode::Allow, root.clone(), Vec::new());
        let files = vec![ProposedFile {
            path: outside_path.join("evil.rs"),
            language: "rust".to_string(),
            content: "bad".to_string(),
        }];
        let outcomes = apply(&sandbox, &files, &root);
        assert_eq!(outcomes[0].status, ApplyStatus::OutsideSandbox);
    }
}
