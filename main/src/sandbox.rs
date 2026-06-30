//! Edit sandbox.
//!
//! Cntx Code confines the assistant's file edits to the project root (and any
//! explicitly allowed write paths) by default, and gates shell/network access
//! through the active permission mode. This prevents an uncontrolled or
//! rogue model run from rewriting files outside the workspace or touching the
//! wider machine.
//!
//! The sandbox is a policy layer consulted by the tool execution loop. It is
//! enabled by default and can be widened with `--allow-write` or disabled
//! entirely with `--dangerously-disable-sandbox`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::permissions::{Mode, Operation, PermissionDecision, PermissionPolicy};

#[derive(Clone, Debug)]
pub struct Sandbox {
    project_root: PathBuf,
    allow_write_roots: Vec<PathBuf>,
    enabled: bool,
    policy: PermissionPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxVerdict {
    pub decision: PermissionDecision,
    pub reason: String,
}

impl Sandbox {
    pub fn new(mode: Mode, project_root: impl Into<PathBuf>, allow_write: Vec<PathBuf>) -> Self {
        let project_root = project_root.into();
        let mut roots = vec![project_root.clone()];
        roots.extend(allow_write);
        Self {
            project_root,
            allow_write_roots: roots,
            enabled: true,
            policy: PermissionPolicy::new(mode),
        }
    }

    /// Disable the sandbox entirely. Only used by `--dangerously-disable-sandbox`.
    pub fn disabled(mode: Mode, project_root: impl Into<PathBuf>) -> Self {
        let mut sandbox = Self::new(mode, project_root, Vec::new());
        sandbox.enabled = false;
        sandbox
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn evaluate(&self, operation: Operation, path: Option<&Path>) -> SandboxVerdict {
        let base = self.policy.decide(operation);
        if !self.enabled {
            return SandboxVerdict {
                decision: base,
                reason: "sandbox disabled".to_string(),
            };
        }

        match operation {
            Operation::ReadFile => SandboxVerdict {
                decision: base,
                reason: "read access permitted by mode".to_string(),
            },
            Operation::WriteFile => {
                let Some(target) = path else {
                    return SandboxVerdict {
                        decision: PermissionDecision::Deny,
                        reason: "write requested without a target path".to_string(),
                    };
                };
                if matches!(base, PermissionDecision::Deny) {
                    return SandboxVerdict {
                        decision: base,
                        reason: "writes blocked by mode".to_string(),
                    };
                }
                if !self.is_within_allowed(target) {
                    return SandboxVerdict {
                        decision: PermissionDecision::Deny,
                        reason: format!(
                            "path {} is outside the sandbox; allow it with --allow-write",
                            target.display()
                        ),
                    };
                }
                SandboxVerdict {
                    decision: base,
                    reason: "write within sandbox".to_string(),
                }
            }
            Operation::Shell | Operation::Network => SandboxVerdict {
                decision: base,
                reason: "gated by permission mode".to_string(),
            },
        }
    }

    /// True when `target` resolves to a location inside one of the allowed
    /// write roots. Non-existent targets are checked against their parent so
    /// new files inside the workspace are permitted.
    pub fn is_within_allowed(&self, target: &Path) -> bool {
        self.allow_write_roots
            .iter()
            .any(|root| is_within(root, target))
    }
}

/// Determines whether `target` is contained within `root`, following symlinks
/// where possible. Returns false if neither path can be resolved.
fn is_within(root: &Path, target: &Path) -> bool {
    let Some(canon_root) = canonicalize(root) else {
        return false;
    };
    let Some(canon_target) = resolve_target(target) else {
        return false;
    };
    canon_target.starts_with(&canon_root)
}

fn canonicalize(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

fn resolve_target(target: &Path) -> Option<PathBuf> {
    if let Some(canon) = canonicalize(target) {
        return Some(canon);
    }
    // New file: resolve its parent and reattach the file name.
    let parent = target.parent()?;
    let file_name = target.file_name()?;
    let canon_parent = canonicalize(parent)?;
    Some(canon_parent.join(file_name))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SandboxSummary {
    pub enabled: bool,
    pub project_root: PathBuf,
    pub allow_write_roots: Vec<PathBuf>,
    pub mode: Mode,
}

impl Sandbox {
    pub fn summary(&self) -> SandboxSummary {
        SandboxSummary {
            enabled: self.enabled,
            project_root: self.project_root.clone(),
            allow_write_roots: self.allow_write_roots.clone(),
            mode: self.policy.mode(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_root() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        let root = temp.path().canonicalize().unwrap();
        (temp, root)
    }

    #[test]
    fn write_inside_root_is_allowed_in_allow_mode() {
        let (_guard, root) = make_root();
        let sandbox = Sandbox::new(Mode::Allow, root.clone(), Vec::new());
        let verdict = sandbox.evaluate(Operation::WriteFile, Some(&root.join("src/lib.rs")));
        assert_eq!(verdict.decision, PermissionDecision::Allow);
    }

    #[test]
    fn write_outside_root_is_denied_even_in_allow_mode() {
        let (_guard, root) = make_root();
        let outside = tempfile::tempdir().unwrap();
        let outside_path = outside.path().canonicalize().unwrap();

        let sandbox = Sandbox::new(Mode::Allow, root, Vec::new());
        let verdict = sandbox.evaluate(Operation::WriteFile, Some(&outside_path.join("evil.rs")));
        assert_eq!(verdict.decision, PermissionDecision::Deny);
        assert!(verdict.reason.contains("outside the sandbox"));
    }

    #[test]
    fn allow_write_extension_lets_extra_root_in() {
        let (_guard, root) = make_root();
        let extra = tempfile::tempdir().unwrap();
        let extra_path = extra.path().canonicalize().unwrap();

        let sandbox = Sandbox::new(Mode::Allow, root, vec![extra_path.clone()]);
        let verdict = sandbox.evaluate(Operation::WriteFile, Some(&extra_path.join("ok.rs")));
        assert_eq!(verdict.decision, PermissionDecision::Allow);
    }

    #[test]
    fn auto_mode_asks_for_writes_inside_root() {
        let (_guard, root) = make_root();
        let sandbox = Sandbox::new(Mode::Auto, root.clone(), Vec::new());
        let verdict = sandbox.evaluate(Operation::WriteFile, Some(&root.join("src/lib.rs")));
        assert_eq!(verdict.decision, PermissionDecision::Ask);
    }

    #[test]
    fn disabled_sandbox_does_not_confine_paths() {
        let (_guard, root) = make_root();
        let outside = tempfile::tempdir().unwrap();
        let outside_path = outside.path().canonicalize().unwrap();

        let sandbox = Sandbox::disabled(Mode::Allow, root);
        let verdict = sandbox.evaluate(Operation::WriteFile, Some(&outside_path.join("evil.rs")));
        assert_eq!(verdict.decision, PermissionDecision::Allow);
        assert!(!sandbox.enabled());
    }

    #[test]
    fn file_only_mode_denies_shell_and_network() {
        let (_guard, root) = make_root();
        let sandbox = Sandbox::new(Mode::FileOnly, root, Vec::new());
        assert_eq!(
            sandbox.evaluate(Operation::Shell, None).decision,
            PermissionDecision::Deny
        );
        assert_eq!(
            sandbox.evaluate(Operation::Network, None).decision,
            PermissionDecision::Deny
        );
    }
}
