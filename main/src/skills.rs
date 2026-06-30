use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::ConfigStore;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub prompt: String,
}

pub struct SkillStore<'a> {
    config_store: &'a ConfigStore,
    project_root: PathBuf,
}

impl<'a> SkillStore<'a> {
    pub fn new(config_store: &'a ConfigStore, project_root: impl Into<PathBuf>) -> Self {
        Self {
            config_store,
            project_root: project_root.into(),
        }
    }

    pub fn create(&self, name: &str, description: &str) -> Result<Skill> {
        self.config_store.ensure_dirs()?;
        let skill = Skill {
            name: name.to_string(),
            description: description.to_string(),
            created_at: Utc::now(),
            prompt: format!(
                "Use this skill when the task matches: {description}\n\nAdd reusable instructions here."
            ),
        };
        fs::write(self.user_skill_path(name), serde_yaml::to_string(&skill)?)?;
        Ok(skill)
    }

    pub fn list(&self) -> Result<Vec<Skill>> {
        let mut skills = Vec::new();
        self.read_dir(self.config_store.skills_dir(), &mut skills)?;
        self.read_dir(self.project_root.join(".cntx").join("skills"), &mut skills)?;
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(skills)
    }

    pub fn get(&self, name: &str) -> Result<Option<Skill>> {
        for skill in self.list()? {
            if skill.name == name {
                return Ok(Some(skill));
            }
        }
        Ok(None)
    }

    fn read_dir(&self, dir: PathBuf, skills: &mut Vec<Skill>) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("yaml") {
                continue;
            }
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("failed to read skill {}", path.display()))?;
            let skill = serde_yaml::from_str(&raw)
                .with_context(|| format!("failed to parse skill {}", path.display()))?;
            skills.push(skill);
        }
        Ok(())
    }

    fn user_skill_path(&self, name: &str) -> PathBuf {
        self.config_store.skills_dir().join(format!("{name}.yaml"))
    }
}
