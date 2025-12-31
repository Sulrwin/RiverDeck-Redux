use std::fs;
use std::path::{Path, PathBuf};

use crate::manifest::PluginManifest;

#[derive(Debug, Clone)]
pub struct InstalledPlugin {
    pub dir: PathBuf,
    pub manifest: PluginManifest,
}

pub fn plugins_dir() -> anyhow::Result<PathBuf> {
    Ok(storage::paths::data_dir()?.join("plugins"))
}

pub fn ensure_plugins_dir() -> anyhow::Result<PathBuf> {
    let dir = plugins_dir()?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn manifest_path(plugin_dir: &Path) -> PathBuf {
    plugin_dir.join("manifest.json")
}

pub fn load_manifest(plugin_dir: &Path) -> anyhow::Result<PluginManifest> {
    let raw = fs::read_to_string(manifest_path(plugin_dir))?;
    let m: PluginManifest = serde_json::from_str(&raw)?;
    Ok(m)
}

pub fn list_installed() -> anyhow::Result<Vec<InstalledPlugin>> {
    let dir = ensure_plugins_dir()?;
    let mut out = vec![];

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match load_manifest(&path) {
            Ok(m) => out.push(InstalledPlugin {
                dir: path,
                manifest: m,
            }),
            Err(_) => continue,
        }
    }

    out.sort_by(|a, b| {
        a.manifest
            .name
            .to_lowercase()
            .cmp(&b.manifest.name.to_lowercase())
    });
    Ok(out)
}

pub fn install_local_dir(src: &Path) -> anyhow::Result<()> {
    let manifest = load_manifest(src)?;
    let dst = ensure_plugins_dir()?.join(&manifest.id);
    if dst.exists() {
        fs::remove_dir_all(&dst)?;
    }
    copy_dir_recursive(src, &dst)?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ty.is_file() {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Install a plugin directory using an atomic replace strategy.
///
/// - Copies `src` into a temporary directory inside the plugins dir
/// - Removes any existing `plugins/<id>`
/// - Renames the temp dir into place
pub fn install_dir_atomic(src: &Path, expected_id: Option<&str>) -> anyhow::Result<()> {
    let manifest = load_manifest(src)?;
    if manifest.id.trim().is_empty() {
        anyhow::bail!("manifest id is empty");
    }
    if let Some(expected) = expected_id {
        if manifest.id != expected {
            anyhow::bail!(
                "manifest id mismatch (expected {expected}, got {})",
                manifest.id
            );
        }
    }

    let plugins_dir = ensure_plugins_dir()?;
    let final_dir = plugins_dir.join(&manifest.id);

    let tmp = tempfile::Builder::new()
        .prefix(&format!(".installing-{}-", manifest.id))
        .tempdir_in(&plugins_dir)?;
    let tmp_path = tmp.keep();

    let res = (|| {
        copy_dir_recursive(src, &tmp_path)?;

        // Best-effort atomic replace. On Windows, rename over existing may fail; remove first.
        if cfg!(windows) && final_dir.exists() {
            let _ = fs::remove_dir_all(&final_dir);
        }
        if final_dir.exists() {
            fs::remove_dir_all(&final_dir)?;
        }

        fs::rename(&tmp_path, &final_dir)?;
        Ok(())
    })();

    if res.is_err() {
        let _ = fs::remove_dir_all(&tmp_path);
    }
    res
}

/// Uninstall a plugin by id (removes `data_dir/plugins/<id>`).
pub fn uninstall(plugin_id: &str) -> anyhow::Result<()> {
    let id = plugin_id.trim();
    if id.is_empty() {
        anyhow::bail!("plugin id is empty");
    }
    let dir = ensure_plugins_dir()?.join(id);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

pub fn plugin_executable_path(plugin: &InstalledPlugin) -> Option<PathBuf> {
    let m = &plugin.manifest;
    let rel = if let Some(exe) = &m.executable {
        Some(exe.as_str())
    } else if cfg!(windows) {
        m.executable_windows.as_deref()
    } else {
        m.executable_linux.as_deref()
    }?;
    Some(plugin.dir.join(rel))
}
