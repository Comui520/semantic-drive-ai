use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// API-first application configuration. Existing config files may still contain
/// retired local-model fields; serde safely ignores them during migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_ai_mode")]
    pub ai_mode: String,
    #[serde(default)]
    pub embedding_api: EmbeddingApiConfig,
    #[serde(default)]
    pub chat_api: ChatApiConfig,
    /// User-selected workspace. None keeps the legacy portable fallback.
    #[serde(default)]
    pub scan_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingApiConfig {
    #[serde(default = "default_embedding_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_embedding_model")]
    pub model: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_timeout_30")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatApiConfig {
    #[serde(default = "default_chat_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_chat_model")]
    pub model: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_timeout_60")]
    pub timeout_secs: u64,
}

fn default_ai_mode() -> String { "api".to_string() }
fn default_embedding_base_url() -> String { "https://api.siliconflow.cn/v1".to_string() }
fn default_chat_base_url() -> String { "https://api.deepseek.com/v1".to_string() }
fn default_embedding_model() -> String { "BAAI/bge-m3".to_string() }
fn default_chat_model() -> String { "deepseek-chat".to_string() }
fn default_true() -> bool { true }
fn default_timeout_30() -> u64 { 30 }
fn default_timeout_60() -> u64 { 60 }

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ai_mode: default_ai_mode(),
            embedding_api: EmbeddingApiConfig::default(),
            chat_api: ChatApiConfig::default(),
            scan_root: None,
        }
    }
}

impl Default for EmbeddingApiConfig {
    fn default() -> Self {
        Self { base_url: default_embedding_base_url(), api_key: String::new(), model: default_embedding_model(), enabled: true, timeout_secs: 30 }
    }
}

impl Default for ChatApiConfig {
    fn default() -> Self {
        Self { base_url: default_chat_base_url(), api_key: String::new(), model: default_chat_model(), enabled: true, timeout_secs: 60 }
    }
}

pub struct ConfigStore {
    path: PathBuf,
    config: AppConfig,
}

impl ConfigStore {
    pub fn load() -> Result<Self, String> {
        let path = Self::config_path()?;
        let config = if path.exists() {
            let content = std::fs::read_to_string(&path).map_err(|e| format!("Failed to read config file: {e}"))?;
            let mut config = serde_json::from_str::<AppConfig>(&content).unwrap_or_else(|e| {
                log::warn!("Config file is invalid ({e}); using API-first defaults");
                AppConfig::default()
            });
            // One-way migration from the former local/API toggle.
            config.ai_mode = default_ai_mode();
            config
        } else {
            AppConfig::default()
        };
        let store = Self { path, config };
        store.persist()?;
        Ok(store)
    }

    /// Store portable user configuration in the OS application-data location,
    /// not beside the executable. This works on installed Windows/macOS/Linux
    /// builds and avoids polluting a scanned folder or removable drive.
    fn config_path() -> Result<PathBuf, String> {
        #[cfg(target_os = "windows")]
        let base = std::env::var_os("APPDATA").map(PathBuf::from)
            .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from));
        #[cfg(target_os = "macos")]
        let base = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Application Support"));
        #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));

        base.map(|dir| dir.join("SemanticDrive").join("config.json"))
            .ok_or_else(|| "Cannot determine an application configuration directory".to_string())
    }

    fn persist(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create config directory: {e}"))?;
        }
        let content = serde_json::to_string_pretty(&self.config).map_err(|e| format!("Failed to serialize config: {e}"))?;
        std::fs::write(&self.path, content).map_err(|e| format!("Failed to write config: {e}"))
    }

    pub fn get_config(&self) -> AppConfig { self.config.clone() }

    pub fn update_config(&mut self, mut config: AppConfig) -> Result<(), String> {
        config.ai_mode = default_ai_mode();
        validate_endpoint(&config.embedding_api.base_url, &config.embedding_api.model, "嵌入")?;
        validate_endpoint(&config.chat_api.base_url, &config.chat_api.model, "聊天")?;
        self.config = config;
        self.persist()
    }

    pub fn set_scan_root(&mut self, root: Option<String>) -> Result<(), String> {
        let mut config = self.config.clone();
        config.scan_root = root;
        self.update_config(config)
    }

    pub fn set_ai_mode(&mut self, _mode: &str) -> Result<(), String> {
        self.config.ai_mode = default_ai_mode();
        self.persist()
    }

    pub fn set_embedding_api_key(&mut self, key: &str) -> Result<(), String> {
        self.config.embedding_api.api_key = key.trim().to_string();
        self.persist()
    }

    pub fn set_chat_api_key(&mut self, key: &str) -> Result<(), String> {
        self.config.chat_api.api_key = key.trim().to_string();
        self.persist()
    }
}

fn validate_endpoint(base_url: &str, model: &str, label: &str) -> Result<(), String> {
    let url = base_url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("{label} API 地址必须以 http:// 或 https:// 开头"));
    }
    if model.trim().is_empty() {
        return Err(format!("{label}模型名称不能为空"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_api_first_configuration() {
        let config = AppConfig::default();
        assert_eq!(config.ai_mode, "api");
        assert!(config.embedding_api.enabled);
        assert!(config.chat_api.enabled);
    }

    #[test]
    fn rejects_invalid_endpoint() {
        assert!(validate_endpoint("example.com", "model", "聊天").is_err());
    }
}
