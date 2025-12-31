use serde::{Deserialize, Serialize};

/// Minimal OpenAction-style manifest model for MVP bring-up.
///
/// This is intentionally scoped:
/// - plugin identity
/// - actions list
/// - settings schema (simple fields)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub actions: Vec<ActionDefinition>,
    /// Relative path to the plugin executable for the current platform.
    ///
    /// For MVP we allow either a single `executable` (cross-platform) or per-OS.
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub executable_linux: Option<String>,
    #[serde(default)]
    pub executable_windows: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub settings: Vec<SettingField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingField {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub ty: SettingType,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingType {
    String,
    Boolean,
    Number,
}
