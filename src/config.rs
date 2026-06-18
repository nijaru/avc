use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct PolicyConfig {
    pub version: u32,
    #[serde(default)]
    pub review: ReviewConfig,
    #[serde(default)]
    pub validation: ValidationConfig,
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub redaction: RedactionConfig,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ReviewConfig {
    #[serde(default = "default_max_files")]
    pub max_files_per_change_warn: u32,
    #[serde(default = "default_max_lines")]
    pub max_lines_per_change_warn: u32,
    #[serde(default)]
    pub require_human_review: bool,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ValidationConfig {
    #[serde(default)]
    pub required_before_export: Vec<ValidationStep>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ValidationStep {
    pub name: String,
    pub command: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AgentsConfig {
    #[serde(default = "default_true")]
    pub allow_agent_assisted_commits: bool,
    #[serde(default)]
    pub require_agent_assisted_trailer: bool,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct RedactionConfig {
    #[serde(default)]
    pub never_commit: Vec<String>,
    #[serde(default)]
    pub store_full_prompts: bool,
    #[serde(default)]
    pub store_tool_outputs: bool,
    #[serde(default = "default_true")]
    pub export_public_summaries: bool,
}

fn default_max_files() -> u32 {
    8
}
fn default_max_lines() -> u32 {
    500
}
fn default_true() -> bool {
    true
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            version: 1,
            review: ReviewConfig::default(),
            validation: ValidationConfig::default(),
            agents: AgentsConfig::default(),
            redaction: RedactionConfig::default(),
        }
    }
}

pub fn default_policy_yaml() -> String {
    let config = PolicyConfig::default();
    serde_yaml::to_string(&config).unwrap_or_default()
}

pub fn write_default_configs(agentvcs_dir: &Path) -> Result<()> {
    let policy = default_policy_yaml();
    std::fs::write(agentvcs_dir.join("policy.yaml"), &policy)
        .context("failed to write policy.yaml")?;

    let agents = r#"version: 1
allow_agent_assisted_commits: true
require_agent_assisted_trailer: false
"#;
    std::fs::write(agentvcs_dir.join("agents.yaml"), agents)
        .context("failed to write agents.yaml")?;

    let validation = r#"version: 1
required_before_export: []
"#;
    std::fs::write(agentvcs_dir.join("validation.yaml"), validation)
        .context("failed to write validation.yaml")?;

    let redaction = r#"version: 1
never_commit:
  - prompts.full_text
  - tool_outputs.raw
  - env
store_full_prompts: false
store_tool_outputs: false
export_public_summaries: true
"#;
    std::fs::write(agentvcs_dir.join("redaction.yaml"), redaction)
        .context("failed to write redaction.yaml")?;

    Ok(())
}
