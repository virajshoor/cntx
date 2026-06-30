use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Auto,
    Counsel,
    Allow,
    RequestPermission,
    FileOnly,
}

impl Mode {
    pub fn description(self) -> &'static str {
        match self {
            Self::Auto => "allow low-risk reads and require approval for writes or shell actions",
            Self::Counsel => "use token-efficient model counsel; permission behavior matches auto",
            Self::Allow => "allow assistant actions without prompting",
            Self::RequestPermission => "ask before any tool or file operation",
            Self::FileOnly => "allow file reads/writes, but block shell and network tools",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ReadFile,
    WriteFile,
    Shell,
    Network,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionDecision {
    Allow,
    Ask,
    Deny,
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

    pub fn decide(&self, operation: Operation) -> PermissionDecision {
        match self.mode {
            Mode::Allow => PermissionDecision::Allow,
            Mode::RequestPermission => PermissionDecision::Ask,
            Mode::FileOnly => match operation {
                Operation::ReadFile | Operation::WriteFile => PermissionDecision::Allow,
                Operation::Shell | Operation::Network => PermissionDecision::Deny,
            },
            Mode::Auto | Mode::Counsel => match operation {
                Operation::ReadFile => PermissionDecision::Allow,
                Operation::WriteFile | Operation::Shell | Operation::Network => {
                    PermissionDecision::Ask
                }
            },
        }
    }
}
