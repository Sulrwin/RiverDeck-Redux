//! Marketplace installer: download plugin archives and install into the local registry.
//!
//! v1 goals:
//! - Support `.zip` and `.tar.gz` / `.tgz` archives
//! - Enforce basic safety (no path traversal, no symlinks)
//! - Validate `manifest.json` and install into `data_dir/plugins/<plugin_id>`

use std::ffi::OsStr;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use crate::manifest::PluginManifest;

#[derive(Debug, Clone, Copy)]
enum ArchiveKind {
    Zip,
    TarGz,
}

/// Download an archive from `url`, extract it safely, validate `manifest.json`, and install it.
///
/// - If `expected_id` is provided, the extracted manifest must match it.
/// - Returns the installed plugin id.
pub async fn install_from_url(url: &str, expected_id: Option<&str>) -> anyhow::Result<String> {
    let url = url.trim();
    if url.is_empty() {
        anyhow::bail!("download url is empty");
    }

    let bytes = crate::marketplace::fetch_bytes(url).await?;
    let kind = detect_archive_kind(url, &bytes)?;

    let expected = expected_id.map(|s| s.to_string());
    let url_owned = url.to_string();

    tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        let staging_root = create_staging_root()?;
        let res: anyhow::Result<String> = (|| {
            extract_archive(kind, &bytes, &staging_root)?;
            let plugin_dir = find_plugin_root(&staging_root)?;
            let manifest = load_manifest(&plugin_dir)?;
            validate_manifest(&manifest, expected.as_deref())?;
            ensure_executable_usable(&plugin_dir, &manifest)?;

            // Install into the standard registry dir (atomic replace).
            crate::registry::install_dir_atomic(&plugin_dir, expected.as_deref())?;
            Ok(manifest.id)
        })();

        // Best-effort cleanup.
        let _ = std::fs::remove_dir_all(&staging_root);

        res.map_err(|e| anyhow::anyhow!("install from url failed ({url_owned}): {e}"))
    })
    .await?
}

fn detect_archive_kind(url: &str, bytes: &[u8]) -> anyhow::Result<ArchiveKind> {
    // ZIP local file header magic: PK\x03\x04
    if bytes.len() >= 4 && &bytes[0..4] == b"PK\x03\x04" {
        return Ok(ArchiveKind::Zip);
    }

    // GZip magic: 1F 8B
    if bytes.len() >= 2 && bytes[0] == 0x1F && bytes[1] == 0x8B {
        return Ok(ArchiveKind::TarGz);
    }

    // Fallback to extension.
    let lower = url.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        return Ok(ArchiveKind::Zip);
    }
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        return Ok(ArchiveKind::TarGz);
    }

    anyhow::bail!("unsupported archive type (expected .zip or .tar.gz/.tgz)");
}

fn create_staging_root() -> anyhow::Result<PathBuf> {
    let base = crate::registry::ensure_plugins_dir()?.join(".staging");
    std::fs::create_dir_all(&base)?;

    // Use tempfile to avoid extra rand dependencies and to reduce collision risk.
    let dir = tempfile::Builder::new()
        .prefix("install-")
        .tempdir_in(&base)?;
    Ok(dir.keep())
}

fn extract_archive(kind: ArchiveKind, bytes: &[u8], staging_root: &Path) -> anyhow::Result<()> {
    match kind {
        ArchiveKind::Zip => extract_zip(bytes, staging_root),
        ArchiveKind::TarGz => extract_tar_gz(bytes, staging_root),
    }
}

fn extract_zip(bytes: &[u8], staging_root: &Path) -> anyhow::Result<()> {
    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();
        let rel = sanitize_rel_path(&name)?;

        // Reject symlinks (zip stores unix mode).
        if let Some(mode) = file.unix_mode() {
            let ty = mode & 0o170000;
            if ty == 0o120000 {
                anyhow::bail!("zip contains symlink entry: {name}");
            }
        }

        let out_path = staging_root.join(&rel);
        if file.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut file, &mut out)?;
    }

    Ok(())
}

fn extract_tar_gz(bytes: &[u8], staging_root: &Path) -> anyhow::Result<()> {
    let cursor = Cursor::new(bytes);
    let gz = flate2::read::GzDecoder::new(cursor);
    let mut ar = tar::Archive::new(gz);

    for entry in ar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let rel = sanitize_rel_components(&path)?;

        let ty = entry.header().entry_type();
        if ty.is_symlink() || ty.is_hard_link() {
            anyhow::bail!("tar contains link entry: {}", rel.display());
        }

        let out_path = staging_root.join(&rel);
        if ty.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if !ty.is_file() {
            // Skip other special types.
            continue;
        }

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out)?;
    }

    Ok(())
}

fn sanitize_rel_path(name: &str) -> anyhow::Result<PathBuf> {
    // Zip uses forward slashes regardless of platform.
    let cleaned = name.trim_start_matches('/');
    if cleaned.is_empty() {
        anyhow::bail!("archive entry has empty path");
    }
    sanitize_rel_components(Path::new(cleaned))
}

fn sanitize_rel_components(p: &Path) -> anyhow::Result<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => {
                anyhow::bail!("archive entry has absolute path: {}", p.display())
            }
            Component::ParentDir => anyhow::bail!("archive entry contains '..': {}", p.display()),
            Component::CurDir => {}
            Component::Normal(seg) => {
                if seg == OsStr::new("") {
                    continue;
                }
                out.push(seg);
            }
        }
    }
    if out.as_os_str().is_empty() {
        anyhow::bail!("archive entry has invalid path: {}", p.display());
    }
    Ok(out)
}

fn find_plugin_root(staging_root: &Path) -> anyhow::Result<PathBuf> {
    // Case 1: manifest at extraction root.
    if staging_root.join("manifest.json").is_file() {
        return Ok(staging_root.to_path_buf());
    }

    // Case 2: single top-level folder contains manifest.
    let mut dirs = vec![];
    for entry in std::fs::read_dir(staging_root)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            dirs.push(p);
        }
    }
    if dirs.len() == 1 && dirs[0].join("manifest.json").is_file() {
        return Ok(dirs.remove(0));
    }

    anyhow::bail!("could not locate plugin root (missing manifest.json)");
}

fn load_manifest(dir: &Path) -> anyhow::Result<PluginManifest> {
    let raw = std::fs::read_to_string(dir.join("manifest.json"))?;
    Ok(serde_json::from_str(&raw)?)
}

fn validate_manifest(manifest: &PluginManifest, expected_id: Option<&str>) -> anyhow::Result<()> {
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
    Ok(())
}

fn ensure_executable_usable(plugin_dir: &Path, manifest: &PluginManifest) -> anyhow::Result<()> {
    let rel = if let Some(exe) = &manifest.executable {
        Some(exe.as_str())
    } else if cfg!(windows) {
        manifest.executable_windows.as_deref()
    } else {
        manifest.executable_linux.as_deref()
    };

    let Some(rel) = rel else {
        // Some plugins may be “manifest-only” for now.
        return Ok(());
    };

    let path = plugin_dir.join(rel);
    if !path.is_file() {
        anyhow::bail!("plugin executable not found: {}", path.display());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&path)?;
        let mut perm = meta.permissions();
        let mode = perm.mode();
        if (mode & 0o111) == 0 {
            perm.set_mode(mode | 0o111);
            std::fs::set_permissions(&path, perm)?;
        }
    }

    Ok(())
}


