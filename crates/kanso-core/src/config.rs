use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineStepSpec {
    pub name: String,
    pub command: String,
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineSpec {
    pub name: String,
    #[serde(default)]
    pub steps: Vec<PipelineStepSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KansoYaml {
    pub trunk_branch: Option<String>,
    #[serde(default)]
    pub ignore_dirs: Vec<String>,
    #[serde(default)]
    pub pipelines: Vec<PipelineSpec>,
}

pub fn load_kanso_yaml(dir: &Path) -> Result<Option<KansoYaml>> {
    let config_path = dir.join(".kanso.yaml");
    if !config_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;
    let parsed: KansoYaml = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", config_path.display()))?;
    Ok(Some(parsed))
}
