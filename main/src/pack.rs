//! Pack large tool outputs for model-visible context while keeping full
//! payloads on disk under the session artifact directory.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Model-visible line/char caps (separate from C stream caps).
pub const MODEL_VISIBLE_LINES: usize = 50;
pub const MODEL_VISIBLE_CHARS: usize = 2_000;

#[derive(Clone, Debug)]
pub struct PackedToolResult {
    pub model_visible: String,
    pub sha256: String,
    pub bytes: usize,
    pub artifact_path: Option<PathBuf>,
}

pub fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Persist full output and return a short stub for the model.
pub fn pack_tool_result(
    tool_name: &str,
    is_error: bool,
    full: &str,
    artifacts_dir: Option<&Path>,
) -> PackedToolResult {
    let bytes = full.len();
    let sha = sha256_hex(full);
    let status = if is_error { "err" } else { "ok" };
    let needs_pack = full.lines().count() > MODEL_VISIBLE_LINES || bytes > MODEL_VISIBLE_CHARS;

    let mut artifact_path = None;
    if needs_pack {
        if let Some(dir) = artifacts_dir {
            let _ = fs::create_dir_all(dir);
            let path = dir.join(format!("{sha}.txt"));
            let _ = fs::write(&path, full);
            artifact_path = Some(path);
        }
    }

    let preview = if needs_pack {
        let mut out = String::new();
        for (i, line) in full.lines().enumerate() {
            if i >= MODEL_VISIBLE_LINES {
                break;
            }
            if out.len() + line.len() + 1 > MODEL_VISIBLE_CHARS {
                break;
            }
            out.push_str(line);
            out.push('\n');
        }
        if out.chars().count() > MODEL_VISIBLE_CHARS {
            out = out.chars().take(MODEL_VISIBLE_CHARS).collect();
        }
        out
    } else {
        full.to_string()
    };

    let pointer = artifact_path
        .as_ref()
        .map(|p| format!(" artifact={}", p.display()))
        .unwrap_or_default();
    let model_visible = if needs_pack {
        format!(
            "tool={tool_name} status={status} bytes={bytes} sha256={sha}{pointer}\n{preview}…[packed; full output on disk]"
        )
    } else {
        format!("tool={tool_name} status={status}\n{preview}")
    };

    PackedToolResult {
        model_visible,
        sha256: sha,
        bytes,
        artifact_path,
    }
}

/// Drop older packed reads of the same path within a window of messages,
/// keeping only the latest. Messages that are not packed reads are kept.
pub fn dedupe_packed_reads(messages: &mut Vec<crate::providers::ChatMessage>) {
    let mut last_path: Option<String> = None;
    let mut last_index: Option<usize> = None;
    let mut drop = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if let Some(path) = packed_read_path(&msg.content) {
            if last_path.as_deref() == Some(path.as_str()) {
                if let Some(prev) = last_index {
                    drop.push(prev);
                }
            }
            last_path = Some(path);
            last_index = Some(i);
        }
    }
    for i in drop.into_iter().rev() {
        messages.remove(i);
    }
}

fn packed_read_path(content: &str) -> Option<String> {
    // Packed read stubs include "tool=read" and often "reading <path>" progress
    // or "File: <path>" from the tool output preview.
    if !content.contains("tool=read") {
        return None;
    }
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("File: ") {
            return Some(rest.trim().to_string());
        }
        if let Some(rest) = line.strip_prefix("reading ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_not_packed() {
        let packed = pack_tool_result("bash", false, "hello\n", None);
        assert!(packed.artifact_path.is_none());
        assert!(packed.model_visible.contains("hello"));
        assert!(!packed.model_visible.contains("[packed"));
    }

    #[test]
    fn large_output_is_packed() {
        let big = "line\n".repeat(200);
        let temp = tempfile::tempdir().unwrap();
        let packed = pack_tool_result("bash", false, &big, Some(temp.path()));
        assert!(packed.artifact_path.is_some());
        assert!(packed.model_visible.contains("sha256="));
        assert!(packed.model_visible.contains("[packed"));
        assert!(packed.model_visible.len() < big.len());
        let stored = std::fs::read_to_string(packed.artifact_path.unwrap()).unwrap();
        assert_eq!(stored, big);
    }
}
