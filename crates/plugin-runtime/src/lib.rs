//! Runtime for executing actions provided by OpenAction plugins.

use std::path::PathBuf;

use openaction::manifest::PluginManifest;
use openaction::registry::{plugin_executable_path, InstalledPlugin};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionInvocation {
    pub plugin_id: String,
    pub action_id: String,
    pub event: String,
    pub key: u8,
    #[serde(default)]
    pub settings: serde_json::Value,
}

pub struct ActionRuntime;

impl ActionRuntime {
    pub fn new() -> Self {
        Self
    }

    pub async fn invoke(
        &self,
        plugin: &InstalledPlugin,
        action: &str,
        key: u8,
        event: &str,
        settings: serde_json::Value,
    ) -> anyhow::Result<()> {
        let exe = plugin_executable_path(plugin)
            .ok_or_else(|| anyhow::anyhow!("plugin has no executable for this platform"))?;

        invoke_process(exe, &plugin.manifest, action, key, event, settings).await
    }
}

impl Default for ActionRuntime {
    fn default() -> Self {
        Self::new()
    }
}

async fn invoke_process(
    exe: PathBuf,
    manifest: &PluginManifest,
    action: &str,
    key: u8,
    event: &str,
    settings: serde_json::Value,
) -> anyhow::Result<()> {
    let payload = ActionInvocation {
        plugin_id: manifest.id.clone(),
        action_id: action.to_string(),
        event: event.to_string(),
        key,
        settings,
    };

    let stdin = serde_json::to_vec(&payload)?;

    let mut child = Command::new(exe)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    if let Some(mut w) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        w.write_all(&stdin).await?;
        w.write_all(b"\n").await?;
    }

    let out = child.wait_with_output().await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        warn!(code=?out.status.code(), %stderr, "plugin action invocation failed");
        anyhow::bail!("plugin invocation failed: {}", out.status);
    }

    Ok(())
}
