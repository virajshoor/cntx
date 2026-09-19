//! Lightweight outline / symbol helpers for bounded project context.
//! Prefer outlines over full-file excerpts unless the user used @file.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const OUTLINE_MAX_LINES: usize = 40;
const OUTLINE_MAX_CHARS: usize = 800;
const SYMBOL_SPAN_LINES: usize = 12;

/// Build a short outline: file head plus signature-looking lines.
pub fn file_outline(path: &Path) -> Option<String> {
    let mut raw = String::new();
    fs::File::open(path)
        .ok()?
        .take(32 * 1024)
        .read_to_string(&mut raw)
        .ok()?;
    let mut lines = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        if i >= OUTLINE_MAX_LINES {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if looks_like_signature(trimmed)
            || (i < 3 && (trimmed.starts_with("//") || trimmed.starts_with('#')))
        {
            lines.push(line);
        }
        if lines.join("\n").chars().count() > OUTLINE_MAX_CHARS {
            break;
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn looks_like_signature(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("fn ")
        || lower.starts_with("pub fn ")
        || lower.starts_with("def ")
        || lower.starts_with("class ")
        || lower.starts_with("struct ")
        || lower.starts_with("enum ")
        || lower.starts_with("impl ")
        || lower.starts_with("interface ")
        || lower.starts_with("type ")
        || lower.starts_with("function ")
        || lower.starts_with("export ")
        || lower.contains(" := func")
}

/// Find a definition-like line for `symbol` under `root` and return a small span.
pub fn symbol_span(root: &Path, symbol: &str) -> Option<(PathBuf, String)> {
    if symbol.is_empty() || symbol.len() > 64 {
        return None;
    }
    let patterns = [
        format!("fn {symbol}"),
        format!("def {symbol}"),
        format!("class {symbol}"),
        format!("struct {symbol}"),
        format!("function {symbol}"),
        format!("{symbol}("),
    ];
    let mut stack = vec![root.to_path_buf()];
    let mut checked = 0usize;
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(
                    name,
                    ".git"
                        | "node_modules"
                        | "target"
                        | ".cache"
                        | "__pycache__"
                        | "dist"
                        | "build"
                ) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            checked += 1;
            if checked > 2_000 {
                return None;
            }
            if crate::blocklist::should_skip(&path) {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            let file_lines: Vec<&str> = contents.lines().collect();
            for (idx, line) in file_lines.iter().enumerate() {
                if patterns.iter().any(|p| line.contains(p)) {
                    let start = idx.saturating_sub(1);
                    let end = (idx + SYMBOL_SPAN_LINES).min(file_lines.len());
                    let span = file_lines[start..end].join("\n");
                    let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                    return Some((relative, span));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_prefers_signatures() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lib.rs");
        std::fs::write(
            &path,
            "// comment\nuse std::io;\n\npub fn answer() -> i32 {\n    42\n}\n",
        )
        .unwrap();
        let outline = file_outline(&path).unwrap();
        assert!(outline.contains("pub fn answer"));
    }
}
