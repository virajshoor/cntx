//! Bounded project context assembly.
//!
//! Cntx keeps prompt augmentation intentionally small: explicit `@file`
//! references first, then a few keyword-matched snippets, plus project memory.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::optimizer::ProjectContextSelector;

const CONTEXT_FILE_LIMIT: usize = 4;
const MAX_EXCERPT_CHARS: usize = 1_200;
const MAX_MEMORY_CHARS: usize = 2_000;
const MAX_CONTEXT_SCAN_BYTES: u64 = 128 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PromptInput {
    pub text: String,
    pub context: PromptContextReport,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PromptContextReport {
    pub memory_included: bool,
    pub memory_chars: usize,
    pub files: Vec<PromptContextFile>,
}

impl PromptContextReport {
    pub fn included_items(&self) -> usize {
        self.files.len() + usize::from(self.memory_included)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PromptContextFile {
    pub path: PathBuf,
    pub score: usize,
    pub excerpt_chars: usize,
    pub source: ContextSource,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextSource {
    Reference,
    Search,
}

/// Build the actual prompt sent to the optimizer and provider.
///
/// When `scan_project` is false, the recursive keyword-based context search is
/// skipped entirely. Only explicit `@file` references and project memory are
/// used. This prevents the CLI from walking an entire home directory or other
/// large tree when the user is not inside a project workspace.
pub fn build_prompt_input(prompt: &str, root: &Path) -> PromptInput {
    build_prompt_input_with_scan(prompt, root, true)
}

/// Build the prompt with explicit control over whether the recursive project
/// context scan runs.
pub fn build_prompt_input_with_scan(prompt: &str, root: &Path, scan_project: bool) -> PromptInput {
    let mut report = PromptContextReport::default();
    let mut sections = Vec::new();

    if let Some(memory) = read_project_memory(root) {
        report.memory_included = true;
        report.memory_chars = memory.chars().count();
        sections.push(format!("Project memory:\n{memory}"));
    }

    if let Some(instructions) = read_project_instructions(root) {
        sections.push(format!("Project instructions:\n{instructions}"));
    }

    // Include a git diff summary when inside a git repo.
    if let Some(diff) = read_git_diff_summary(root) {
        sections.push(format!("Current git changes:\n{diff}"));
    }

    let mut seen = BTreeSet::new();
    for path in referenced_paths(prompt) {
        if report.files.len() >= CONTEXT_FILE_LIMIT {
            break;
        }
        // Handle @directory/ references: include a file tree summary.
        let full = root.join(&path);
        if full.is_dir() {
            if let Some(tree) = directory_tree(&full, root) {
                seen.insert(path.clone());
                report.files.push(PromptContextFile {
                    path: path.clone(),
                    score: usize::MAX,
                    excerpt_chars: tree.chars().count(),
                    source: ContextSource::Reference,
                });
                sections.push(format!(
                    "Directory tree `{}`:\n```text\n{}\n```",
                    path.display(),
                    tree
                ));
            }
            continue;
        }
        if let Some((relative, excerpt)) = read_context_file(root, &path) {
            seen.insert(relative.clone());
            report.files.push(PromptContextFile {
                path: relative.clone(),
                score: usize::MAX,
                excerpt_chars: excerpt.chars().count(),
                source: ContextSource::Reference,
            });
            sections.push(format!(
                "Referenced file `{}`:\n```text path={}\n{}\n```",
                relative.display(),
                relative.display(),
                excerpt.trim_end()
            ));
        }
    }

    if scan_project && report.files.len() < CONTEXT_FILE_LIMIT {
        let remaining = CONTEXT_FILE_LIMIT - report.files.len();
        if let Ok(candidates) = ProjectContextSelector::new(root).select(prompt, remaining) {
            for candidate in candidates {
                let relative = relative_path(root, &candidate.path);
                if seen.contains(&relative) {
                    continue;
                }
                seen.insert(relative.clone());
                let excerpt = trim_chars(&candidate.excerpt, MAX_EXCERPT_CHARS);
                report.files.push(PromptContextFile {
                    path: relative.clone(),
                    score: candidate.score,
                    excerpt_chars: excerpt.chars().count(),
                    source: ContextSource::Search,
                });
                sections.push(format!(
                    "Relevant file `{}` (score {}):\n```text path={}\n{}\n```",
                    relative.display(),
                    candidate.score,
                    relative.display(),
                    excerpt.trim_end()
                ));
            }
        }
    }

    if sections.is_empty() {
        return PromptInput {
            text: prompt.to_string(),
            context: report,
        };
    }

    let text = format!(
        "Use the bounded project context below when it is relevant. Do not assume omitted files are irrelevant.\n\n{}\n\nUser request:\n{}",
        sections.join("\n\n"),
        prompt
    );
    PromptInput {
        text,
        context: report,
    }
}

fn read_project_memory(root: &Path) -> Option<String> {
    let memory_path = root.join(".cntx").join("memory.md");
    let raw = fs::read_to_string(memory_path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trim_chars(trimmed, MAX_MEMORY_CHARS))
}

/// Read project-level instructions from AGENTS.md or .cntx/instructions.md.
/// These are injected as a system-like section so the model follows
/// project-specific conventions.
fn read_project_instructions(root: &Path) -> Option<String> {
    const MAX_INSTRUCTIONS_CHARS: usize = 4_000;
    for path in [
        root.join("AGENTS.md"),
        root.join(".cntx").join("instructions.md"),
    ] {
        if let Ok(raw) = fs::read_to_string(&path) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Some(trim_chars(trimmed, MAX_INSTRUCTIONS_CHARS));
            }
        }
    }
    None
}

/// Run `git diff --stat` in the project root and return the output as a
/// short summary. Returns None when not in a git repo or no changes.
fn read_git_diff_summary(root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("diff")
        .arg("--stat")
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trim_chars(trimmed, 2_000))
}

fn referenced_paths(prompt: &str) -> Vec<PathBuf> {
    prompt
        .split_whitespace()
        .filter_map(|token| token.strip_prefix('@'))
        .map(|token| token.trim_matches(reference_trim_char))
        .filter(|token| !token.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn reference_trim_char(ch: char) -> bool {
    matches!(ch, ',' | '.' | ':' | ';' | ')' | ']' | '}' | '\'' | '"')
}

/// Build a compact file tree for a directory, limited to ~60 entries and
/// depth 3. Excludes common ignored directories.
fn directory_tree(dir: &Path, root: &Path) -> Option<String> {
    const MAX_ENTRIES: usize = 60;
    const MAX_DEPTH: usize = 3;
    let mut lines = Vec::new();
    collect_tree(dir, root, 0, MAX_DEPTH, MAX_ENTRIES, &mut lines);
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn collect_tree(
    dir: &Path,
    _root: &Path,
    depth: usize,
    max_depth: usize,
    max_entries: usize,
    lines: &mut Vec<String>,
) {
    if depth > max_depth || lines.len() >= max_entries {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut items: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    items.sort_by_key(|e| e.file_name());
    for entry in items {
        if lines.len() >= max_entries {
            return;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if matches!(
            name_str.as_ref(),
            ".git"
                | "target"
                | "node_modules"
                | "dist"
                | "build"
                | "coverage"
                | ".cache"
                | "__pycache__"
                | ".venv"
                | "venv"
        ) {
            continue;
        }
        let indent = "  ".repeat(depth);
        let path = entry.path();
        if path.is_dir() {
            lines.push(format!("{indent}{}/", name_str));
            collect_tree(&path, _root, depth + 1, max_depth, max_entries, lines);
        } else {
            lines.push(format!("{indent}{name_str}"));
        }
    }
}

fn read_context_file(root: &Path, requested: &Path) -> Option<(PathBuf, String)> {
    if requested.is_absolute() || !is_safe_context_path(requested) {
        return None;
    }
    let path = root.join(requested);
    let canonical_root = fs::canonicalize(root).ok()?;
    let canonical_path = fs::canonicalize(&path).ok()?;
    if !canonical_path.starts_with(&canonical_root) || !canonical_path.is_file() {
        return None;
    }
    let excerpt = read_prefix(&canonical_path).ok()?;
    let relative = canonical_path
        .strip_prefix(&canonical_root)
        .unwrap_or(&canonical_path)
        .to_path_buf();
    Some((relative, trim_chars(&excerpt, MAX_EXCERPT_CHARS)))
}

fn read_prefix(path: &Path) -> std::io::Result<String> {
    let mut contents = String::new();
    fs::File::open(path)?
        .take(MAX_CONTEXT_SCAN_BYTES)
        .read_to_string(&mut contents)?;
    Ok(contents)
}

fn is_safe_context_path(path: &Path) -> bool {
    !crate::blocklist::is_secret_file(path)
}

fn relative_path(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

fn trim_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_input_includes_project_memory_and_referenced_file() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".cntx")).unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join(".cntx/memory.md"), "- prefer small diffs").unwrap();
        fs::write(
            temp.path().join("src/lib.rs"),
            "pub fn answer() -> i32 { 42 }\n",
        )
        .unwrap();

        let input = build_prompt_input("review @src/lib.rs", temp.path());

        assert!(input.context.memory_included);
        assert_eq!(input.context.files.len(), 1);
        assert!(input.text.contains("Project memory"));
        assert!(input.text.contains("Referenced file `src/lib.rs`"));
    }

    #[test]
    fn prompt_input_does_not_include_secret_reference() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("secrets.yaml"), "keys: {openai: sk-test}").unwrap();

        let input = build_prompt_input("read @secrets.yaml", temp.path());

        assert!(input.context.files.is_empty());
        assert!(!input.text.contains("sk-test"));
    }
}
