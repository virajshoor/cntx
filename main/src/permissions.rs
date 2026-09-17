//! Permission modes and approval policy.
//!
//! Mode semantics live in the C core (`csrc/permissions.c`) so every caller
//! — interactive sessions, one-shot prompts, apply mode, counsel mode, and
//! tests — shares one decision table. This module provides the Rust types,
//! clap/serde names with legacy aliases, and the interactive approval prompt.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Canonical CLI/serde mode names: `auto-approve`, `all-approve`,
/// `manual-approve`, plus legacy `counsel` and `file-only`. The legacy
/// names `auto`, `allow`, and `request-permission` remain accepted aliases.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    #[value(name = "auto-approve", alias = "auto")]
    #[serde(rename = "auto-approve", alias = "auto")]
    Auto,
    Counsel,
    #[value(name = "all-approve", alias = "allow")]
    #[serde(rename = "all-approve", alias = "allow")]
    Allow,
    #[value(name = "manual-approve", alias = "request-permission")]
    #[serde(rename = "manual-approve", alias = "request-permission")]
    RequestPermission,
    FileOnly,
}

impl Mode {
    /// Mode value in the C core decision table.
    pub fn code(self) -> i32 {
        match self {
            Self::Auto => crate::core::MODE_AUTO_APPROVE,
            Self::Counsel => crate::core::MODE_COUNSEL,
            Self::Allow => crate::core::MODE_ALL_APPROVE,
            Self::RequestPermission => crate::core::MODE_MANUAL_APPROVE,
            Self::FileOnly => crate::core::MODE_FILE_ONLY,
        }
    }

    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            crate::core::MODE_AUTO_APPROVE => Some(Self::Auto),
            crate::core::MODE_COUNSEL => Some(Self::Counsel),
            crate::core::MODE_ALL_APPROVE => Some(Self::Allow),
            crate::core::MODE_MANUAL_APPROVE => Some(Self::RequestPermission),
            crate::core::MODE_FILE_ONLY => Some(Self::FileOnly),
            _ => None,
        }
    }

    /// Canonical CLI name, from the C core.
    pub fn as_str(self) -> &'static str {
        crate::core::mode_canonical_name(self.code()).unwrap_or("auto-approve")
    }

    /// Shift+Tab cycle order, from the C core.
    pub fn next(self) -> Self {
        Self::from_code(crate::core::mode_next(self.code())).unwrap_or(Self::Auto)
    }

    /// Parse a canonical or legacy alias name through the C core.
    pub fn parse(name: &str) -> Option<Self> {
        crate::core::mode_parse(name).and_then(Self::from_code)
    }

    pub fn description(self) -> &'static str {
        crate::core::mode_description(self.code()).unwrap_or("")
    }
}

/// Approval is explicit and fail-closed, including redirected stdin and EOF.
/// Prompts use plain language ("Cntx wants to: ...") so non-technical users
/// can decide without reading raw tool JSON.
pub fn confirm(action: &str) -> bool {
    use std::io::{self, IsTerminal, Write};
    if !io::stdin().is_terminal() {
        eprintln!(
            "Cntx needs your permission to {action}, but this is not an interactive terminal. Rerun interactively, or use --mode all-approve to allow permitted tools without prompting."
        );
        return false;
    }
    eprintln!("Cntx wants to: {action}");
    eprint!("Allow once? [y = yes / n = no] ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).is_ok()
        && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ReadFile,
    WriteFile,
    Shell,
    Network,
}

impl Operation {
    fn code(self) -> i32 {
        match self {
            Self::ReadFile => crate::core::OP_READ,
            Self::WriteFile => crate::core::OP_WRITE,
            Self::Shell => crate::core::OP_SHELL,
            Self::Network => crate::core::OP_NETWORK,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionDecision {
    Allow,
    Ask,
    Deny,
}

impl PermissionDecision {
    fn from_code(code: i32) -> Self {
        match code {
            crate::core::DECISION_ASK => Self::Ask,
            crate::core::DECISION_DENY => Self::Deny,
            _ => Self::Allow,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PermissionPolicy {
    mode: Mode,
}

impl PermissionPolicy {
    pub fn new(mode: Mode) -> Self {
        Self { mode }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Delegate the decision to the C core. This is the single policy
    /// implementation shared by the tool loop, apply mode, and counsel.
    pub fn decide(&self, operation: Operation) -> PermissionDecision {
        PermissionDecision::from_code(crate::core::permission_decide(
            self.mode.code(),
            operation.code(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_names_come_from_c_core() {
        assert_eq!(Mode::Auto.as_str(), "auto-approve");
        assert_eq!(Mode::Allow.as_str(), "all-approve");
        assert_eq!(Mode::RequestPermission.as_str(), "manual-approve");
        assert_eq!(Mode::Counsel.as_str(), "counsel");
        assert_eq!(Mode::FileOnly.as_str(), "file-only");
    }

    #[test]
    fn legacy_aliases_resolve() {
        assert_eq!(Mode::parse("auto"), Some(Mode::Auto));
        assert_eq!(Mode::parse("allow"), Some(Mode::Allow));
        assert_eq!(
            Mode::parse("request-permission"),
            Some(Mode::RequestPermission)
        );
        assert_eq!(Mode::parse("auto-approve"), Some(Mode::Auto));
        assert_eq!(Mode::parse("all-approve"), Some(Mode::Allow));
        assert_eq!(Mode::parse("manual-approve"), Some(Mode::RequestPermission));
    }

    #[test]
    fn cycle_matches_contract() {
        assert_eq!(Mode::Auto.next(), Mode::Allow);
        assert_eq!(Mode::Allow.next(), Mode::RequestPermission);
        assert_eq!(Mode::RequestPermission.next(), Mode::Auto);
        assert_eq!(Mode::Counsel.next(), Mode::Auto);
        assert_eq!(Mode::FileOnly.next(), Mode::Auto);
    }

    #[test]
    fn policy_delegates_to_c() {
        let policy = PermissionPolicy::new(Mode::Auto);
        assert_eq!(
            policy.decide(Operation::ReadFile),
            PermissionDecision::Allow
        );
        assert_eq!(policy.decide(Operation::Shell), PermissionDecision::Ask);
        let file_only = PermissionPolicy::new(Mode::FileOnly);
        assert_eq!(file_only.decide(Operation::Shell), PermissionDecision::Deny);
    }
}
