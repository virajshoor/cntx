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
pub fn build_prompt_input(prompt: &str, root: &Path) -> PromptInput {
    let mut report = PromptContextReport::default();
    let mut sections = Vec::new();

    if let Some(memory) = read_project_memory(root) {
        report.memory_included = true;
        report.memory_chars = memory.chars().count();
        sections.push(format!("Project memory:\n{memory}"));
    }

    let mut seen = BTreeSet::new();
    for path in referenced_paths(prompt) {
        if report.files.len() >= CONTEXT_FILE_LIMIT {
            break;
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

    if report.files.len() < CONTEXT_FILE_LIMIT {
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
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let lower = name.to_lowercase();
    !matches!(
        lower.as_str(),
        ".env"
            | ".env.local"
            | ".env.production"
            | ".npmrc"
            | ".pypirc"
            | "secrets.yaml"
            | "secrets.yml"
            | "credentials"
            | "credentials.json"
            | "id_rsa"
            | "id_ed25519"
    )
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
