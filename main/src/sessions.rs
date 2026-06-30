use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::ConfigStore;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<SessionMessage>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    pub at: DateTime<Utc>,
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

impl<'a> SessionStore<'a> {
    pub fn new(config_store: &'a ConfigStore) -> Self {
        Self { config_store }
    }

    pub fn save(&self, session: &Session) -> Result<()> {
        self.config_store.ensure_dirs()?;
        fs::write(self.path(&session.id), serde_yaml::to_string(session)?)?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Session> {
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
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
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
        self.save(&session)?;
        Ok(session)
    }

    fn path(&self, id: &str) -> std::path::PathBuf {
        self.config_store.sessions_dir().join(format!("{id}.yaml"))
    }
}
