use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

const INITIAL_PROMPT_BUFFER_CAP: usize = 64 * 1024;
const MAX_CONTEXT_SCAN_BYTES: u64 = 128 * 1024;
const MAX_CONTEXT_EXCERPT_CHARS: usize = 800;

/// Maximum directory depth for the recursive context scan. Prevents walking
/// deep nested trees.
const MAX_SCAN_DEPTH: usize = 10;

/// Maximum number of files to inspect during the recursive context scan.
/// Prevents hanging on very large projects.
const MAX_SCAN_FILES: usize = 5_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OptimizationReport {
    pub original_chars: usize,
    pub optimized_chars: usize,
    pub estimated_tokens: usize,
    pub duplicate_lines_removed: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OptimizedPrompt {
    pub text: String,
    pub report: OptimizationReport,
}

#[derive(Clone, Debug, Default)]
pub struct PromptOptimizer;

impl PromptOptimizer {
    pub fn optimize(&self, prompt: &str) -> OptimizedPrompt {
        let mut seen = HashSet::new();
        let mut duplicate_lines_removed = 0;
        let mut text = String::with_capacity(prompt.len().min(INITIAL_PROMPT_BUFFER_CAP));
        let mut in_code_fence = false;
        let mut pending_blank = false;
        let mut emitted_any = false;

        for line in prompt.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                in_code_fence = !in_code_fence;
                push_collapsed_line(
                    &mut text,
                    line.trim_end(),
                    &mut pending_blank,
                    &mut emitted_any,
                );
                continue;
            }

            if !in_code_fence && !trimmed.is_empty() {
                let normalized = normalize_whitespace(trimmed);
                if normalized.len() > 24 && !seen.insert(normalized.clone()) {
                    duplicate_lines_removed += 1;
                    continue;
                }
                push_collapsed_line(&mut text, &normalized, &mut pending_blank, &mut emitted_any);
            } else {
                push_collapsed_line(
                    &mut text,
                    line.trim_end(),
                    &mut pending_blank,
                    &mut emitted_any,
                );
            }
        }

        let estimated_tokens = estimate_tokens(&text);
        OptimizedPrompt {
            report: OptimizationReport {
                original_chars: prompt.len(),
                optimized_chars: text.len(),
                estimated_tokens,
                duplicate_lines_removed,
            },
            text,
        }
    }
}

pub fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    let words = text.split_whitespace().count();
    chars.div_ceil(4).max(words)
}

fn normalize_whitespace(line: &str) -> String {
    let mut normalized = String::with_capacity(line.len());
    let mut pending_space = false;

    for part in line.split_whitespace() {
        if pending_space {
            normalized.push(' ');
        }
        normalized.push_str(part);
        pending_space = true;
    }

    normalized
}

fn push_collapsed_line(
    output: &mut String,
    line: &str,
    pending_blank: &mut bool,
    emitted_any: &mut bool,
) {
    if line.trim().is_empty() {
        if *emitted_any {
            *pending_blank = true;
        }
        return;
    }

    if *pending_blank && !output.is_empty() {
        output.push('\n');
    }
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(line);
    *pending_blank = false;
    *emitted_any = true;
}

#[derive(Clone, Debug)]
pub struct ContextCandidate {
    pub path: PathBuf,
    pub score: usize,
    pub excerpt: String,
}

#[derive(Clone, Debug)]
pub struct ProjectContextSelector {
    root: PathBuf,
}

impl ProjectContextSelector {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn select(&self, prompt: &str, limit: usize) -> Result<Vec<ContextCandidate>> {
        let terms = prompt_terms(prompt);
        if terms.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let mut candidates = Vec::new();
        visit_files(&self.root, &mut |path| {
            if candidates.len() > limit * 8 {
                return Ok(());
            }
            if should_skip(path) {
                return Ok(());
            }
            let Ok(contents) = read_context_prefix(path) else {
                return Ok(());
            };
            let lower = contents.to_lowercase();
            let score = terms.iter().filter(|term| lower.contains(*term)).count();
            if score > 0 {
                candidates.push(ContextCandidate {
                    path: path.to_path_buf(),
                    score,
                    excerpt: contents.chars().take(MAX_CONTEXT_EXCERPT_CHARS).collect(),
                });
            }
            Ok(())
        })?;

        candidates.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
        candidates.truncate(limit);
        Ok(candidates)
    }
}

fn prompt_terms(prompt: &str) -> Vec<String> {
    prompt
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
        .filter(|term| term.len() > 3)
        .map(|term| term.to_lowercase())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

fn visit_files(root: &Path, visitor: &mut dyn FnMut(&Path) -> Result<()>) -> Result<()> {
    let mut file_count = 0usize;
    visit_files_bounded(root, visitor, 0, &mut file_count)
}

fn visit_files_bounded(
    dir: &Path,
    visitor: &mut dyn FnMut(&Path) -> Result<()>,
    depth: usize,
    file_count: &mut usize,
) -> Result<()> {
    if depth > MAX_SCAN_DEPTH || *file_count > MAX_SCAN_FILES {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if matches!(
            file_name,
            ".git"
                | ".cntx"
                | ".cache"
                | ".next"
                | "target"
                | "node_modules"
                | "dist"
                | "build"
                | "coverage"
                | "vendor"
                | ".npm"
                | ".cargo"
                | ".rustup"
                | "Library"
                | ".config"
                | ".local"
                | "__pycache__"
                | ".venv"
                | "venv"
                | ".tox"
                | ".mypy_cache"
                | ".pytest_cache"
        ) {
            continue;
        }
        if path.is_dir() {
            visit_files_bounded(&path, visitor, depth + 1, file_count)?;
        } else if path.is_file() {
            *file_count += 1;
            if *file_count > MAX_SCAN_FILES {
                return Ok(());
            }
            visitor(&path)?;
        }
    }
    Ok(())
}

fn should_skip(path: &Path) -> bool {
    crate::blocklist::should_skip(path)
}

fn read_context_prefix(path: &Path) -> Result<String> {
    let mut contents = String::new();
    File::open(path)?
        .take(MAX_CONTEXT_SCAN_BYTES)
        .read_to_string(&mut contents)?;
    Ok(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimizer_removes_duplicate_natural_language_lines() {
        let optimizer = PromptOptimizer;
        let prompt = "please inspect the same repeated requirement carefully\nplease inspect the same repeated requirement carefully\n```rust\nlet x = 1;\nlet x = 1;\n```";
        let optimized = optimizer.optimize(prompt);

        assert_eq!(optimized.report.duplicate_lines_removed, 1);
        assert!(optimized.text.contains("let x = 1;\nlet x = 1;"));
    }

    #[test]
    fn token_estimate_is_never_zero_for_text() {
        assert!(estimate_tokens("hello") > 0);
    }

    #[test]
    fn optimizer_collapses_blank_lines_without_intermediate_join() {
        let optimizer = PromptOptimizer;
        let optimized = optimizer.optimize("\n\nalpha\n\n\nbeta\n");

        assert_eq!(optimized.text, "alpha\n\nbeta");
    }

    #[test]
    fn context_selector_respects_limit_zero() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("lib.rs"), "router provider endpoint").unwrap();

        let selector = ProjectContextSelector::new(temp.path());
        let selected = selector.select("router", 0).unwrap();

        assert!(selected.is_empty());
    }

    #[test]
    fn context_selector_reads_bounded_prefix_of_large_files() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("large.rs"),
            format!("router{}", "x".repeat(MAX_CONTEXT_SCAN_BYTES as usize + 1)),
        )
        .unwrap();

        let selector = ProjectContextSelector::new(temp.path());
        let selected = selector.select("router", 10).unwrap();

        assert_eq!(selected.len(), 1);
        assert!(selected[0].path.ends_with("large.rs"));
        assert!(selected[0].excerpt.len() <= MAX_CONTEXT_EXCERPT_CHARS);
    }
}
