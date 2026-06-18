use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// avc configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_tracking")]
    pub tracking: TrackingConfig,
    #[serde(default = "default_save")]
    pub save: SaveConfig,
    #[serde(default = "default_run")]
    pub run: RunConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackingConfig {
    /// Whether to auto-commit on invocation
    pub auto: bool,
    /// Patterns to ignore beyond .gitignore
    #[serde(default)]
    pub ignore: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveConfig {
    /// Whether to add avc trailers to saved commits
    pub trailers: bool,
    /// Whether to auto-generate messages for saves without -m
    pub auto_message: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    /// Only snapshot after if command succeeded
    pub snapshot_on_success: bool,
}

fn default_tracking() -> TrackingConfig {
    TrackingConfig {
        auto: true,
        ignore: Vec::new(),
    }
}

fn default_save() -> SaveConfig {
    SaveConfig {
        trailers: true,
        auto_message: true,
    }
}

fn default_run() -> RunConfig {
    RunConfig {
        snapshot_on_success: true,
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tracking: default_tracking(),
            save: default_save(),
            run: default_run(),
        }
    }
}

/// Path to the config file.
pub fn config_path(repo_root: &Path) -> std::path::PathBuf {
    repo_root.join(".avc").join("config")
}

/// Load config from .avc/config. Returns default if file doesn't exist.
#[allow(dead_code)]
pub fn load(repo_root: &Path) -> Result<Config> {
    let path = config_path(repo_root);
    if !path.exists() {
        return Ok(Config::default());
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;

    let config: Config = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse config: {}", path.display()))?;

    Ok(config)
}

/// Write default config to .avc/config.
pub fn write_default(repo_root: &Path) -> Result<()> {
    let path = config_path(repo_root);
    let config = Config::default();
    let content = serde_yaml::to_string(&config)?;
    std::fs::write(&path, content)?;
    Ok(())
}
