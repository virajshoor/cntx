use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::ConfigStore;

/// A persistent goal attached to a session. Status transitions and step
/// limits are decided by the C core state machine; this struct only stores
/// state. Goal status survives compaction and session reload.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GoalState {
    pub objective: String,
    /// active | paused | blocked | completed | cancelled
    pub status: String,
    #[serde(default)]
    pub progress: String,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub steps_used: u32,
    #[serde(default)]
    pub max_steps: u32,
}

impl GoalState {
    pub fn new(objective: impl Into<String>) -> Self {
        Self {
            objective: objective.into(),
            status: "active".to_string(),
            progress: String::new(),
            evidence: String::new(),
            steps_used: 0,
            max_steps: crate::core::goal_default_max_steps(),
        }
    }

    pub fn status_code(&self) -> i32 {
        crate::core::goal_parse_status(&self.status).unwrap_or(crate::core::GOAL_NONE)
    }

    pub fn set_status(&mut self, code: i32) {
        if let Some(name) = crate::core::goal_status_name(code) {
            self.status = name.to_string();
        }
    }

    /// True when the goal is running (active), meaning `goal_update` is
    /// accepted and the loop keeps going until paused/blocked/done.
    pub fn is_active(&self) -> bool {
        self.status_code() == crate::core::GOAL_ACTIVE
    }

    /// True when the goal still exists and is not finished, so replacing it
    /// with a new objective is rejected until cancel/completion.
    pub fn is_replaceable(&self) -> bool {
        matches!(
            self.status_code(),
            crate::core::GOAL_COMPLETED | crate::core::GOAL_CANCELLED
        )
    }
}

/// Canonical status names accepted by `/goal` display and parsing.
pub fn goal_status_names() -> &'static [&'static str] {
    &["active", "paused", "blocked", "completed", "cancelled"]
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<SessionMessage>,
    /// Workspace the session belongs to. Sessions load only in their own
    /// workspace; legacy sessions without one use the current workspace
    /// with a visible notice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<PathBuf>,
    /// Rolling compaction summary. Old messages stay on disk; requests omit
    /// messages before `context_start_index` and include this summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Index of the first message NOT covered by the summary. Zero when no
    /// summary exists.
    #[serde(default)]
    pub context_start_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<GoalState>,
}

impl Session {
    pub fn new(title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            workspace_root: std::env::current_dir().ok(),
            summary: None,
            context_start_index: 0,
            goal: None,
        }
    }

    pub fn push(&mut self, role: impl Into<String>, content: impl Into<String>) {
        self.messages.push(SessionMessage {
            role: role.into(),
            content: content.into(),
            at: Utc::now(),
        });
        self.updated_at = Utc::now();
    }
}

pub struct SessionStore<'a> {
    config_store: &'a ConfigStore,
}

/// Session ids become filenames. Untrusted ids (imports, `/resume` input)
/// must never escape the sessions directory through separators, traversal,
/// or hidden/flag-like names.
pub fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.contains('\0')
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains("..")
        && !id.starts_with('.')
        && !id.starts_with('-')
}

impl<'a> SessionStore<'a> {
    pub fn new(config_store: &'a ConfigStore) -> Self {
        Self { config_store }
    }

    /// Save with atomic replacement so a crash never leaves a half-written
    /// session file behind.
    pub fn save(&self, session: &Session) -> Result<()> {
        if !is_valid_session_id(&session.id) {
            anyhow::bail!("invalid session id '{}': not a safe filename", session.id);
        }
        self.config_store.ensure_dirs()?;
        let path = self.path(&session.id);
        let raw = serde_yaml::to_string(session)?;
        let temp_path = self.config_store.sessions_dir().join(format!(
            "{}.tmp-{}",
            session.id,
            std::process::id()
        ));
        fs::write(&temp_path, raw)
            .with_context(|| format!("failed to write session {}", temp_path.display()))?;
        fs::rename(&temp_path, &path)
            .with_context(|| format!("failed to replace session {}", path.display()))?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Session> {
        if !is_valid_session_id(id) {
            anyhow::bail!("invalid session id '{id}': not a safe filename");
        }
        let path = self.path(id);
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read session {}", path.display()))?;
        let session = serde_yaml::from_str(&raw)
            .with_context(|| format!("failed to parse session {}", path.display()))?;
        Ok(session)
    }

    pub fn list(&self) -> Result<Vec<Session>> {
        self.config_store.ensure_dirs()?;
        let mut sessions = Vec::new();
        for entry in fs::read_dir(self.config_store.sessions_dir())? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("yaml") {
                continue;
            }
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(session) = serde_yaml::from_str::<Session>(&raw) {
                    sessions.push(session);
                }
            }
        }
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
        Ok(sessions)
    }

    pub fn latest(&self) -> Result<Option<Session>> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn export(&self, id: &str, output: &Path) -> Result<()> {
        let session = self.load(id)?;
        fs::write(output, serde_json::to_string_pretty(&session)?)?;
        Ok(())
    }

    pub fn import(&self, input: &Path) -> Result<Session> {
        let raw = fs::read_to_string(input)
            .with_context(|| format!("failed to read session import {}", input.display()))?;
        let mut session: Session = serde_json::from_str(&raw)
            .or_else(|_| serde_yaml::from_str(&raw))
            .with_context(|| format!("failed to parse session import {}", input.display()))?;
        if session.id.trim().is_empty() {
            session.id = Uuid::new_v4().to_string();
        }
        // Untrusted imported ids are replaced rather than trusted as
        // filenames; the import still succeeds with a fresh id.
        if !is_valid_session_id(&session.id) {
            let old_id = session.id.clone();
            session.id = Uuid::new_v4().to_string();
            eprintln!(
                "warning: session id '{old_id}' is not a safe filename; imported with new id '{}'",
                session.id
            );
        }
        // If a session with the same ID already exists, generate a new ID to
        // avoid silently overwriting the user's existing session.
        let existing_path = self.path(&session.id);
        if existing_path.exists() {
            let old_id = session.id.clone();
            session.id = Uuid::new_v4().to_string();
            eprintln!(
                "warning: session id '{}' already exists; imported with new id '{}'",
                old_id, session.id
            );
        }
        self.save(&session)?;
        Ok(session)
    }

    fn path(&self, id: &str) -> PathBuf {
        self.config_store.sessions_dir().join(format!("{id}.yaml"))
    }
}
