use std::path::PathBuf;

use directories::ProjectDirs;

pub fn project_dirs() -> anyhow::Result<ProjectDirs> {
    ProjectDirs::from("io", "github", "riverdeck-redux")
        .ok_or_else(|| anyhow::anyhow!("unable to determine platform data directories"))
}

/// Directory for user-writable application data (profiles, logs, caches).
pub fn data_dir() -> anyhow::Result<PathBuf> {
    Ok(project_dirs()?.data_dir().to_path_buf())
}
