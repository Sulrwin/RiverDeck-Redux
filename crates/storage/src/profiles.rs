use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use app_core::ids::ProfileId;
use serde::{Deserialize, Serialize};

use crate::paths;

const PROFILE_SCHEMA_VERSION: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub version: u32,
    pub id: ProfileId,
    pub name: String,
    pub key_count: u8,
    pub keys: Vec<KeyConfig>,
    /// Stream Deck+ has 4 dials; other devices will typically leave this empty.
    #[serde(default)]
    pub dials: Vec<DialConfig>,
    /// Stream Deck+ touch strip config; defaults to an empty config for non-plus devices.
    #[serde(default)]
    pub touch_strip: TouchStripConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Background {
    None,
    Solid { rgb: [u8; 3] },
}

impl Default for Background {
    fn default() -> Self {
        Background::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Appearance {
    #[serde(default)]
    pub background: Background,
    /// Optional path to an icon image (PNG/JPEG). UI/runtime resolve this on render.
    #[serde(default)]
    pub icon_path: Option<String>,
    /// Optional single-line text rendered on the LCD surface.
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeyConfig {
    /// Temporary, MVP-level metadata for UI bring-up.
    /// OpenAction bindings will later live alongside this.
    pub label: String,
    #[serde(default)]
    pub action: Option<actions::ActionBinding>,
    #[serde(default)]
    pub appearance: Appearance,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DialConfig {
    #[serde(default)]
    pub label: String,
    /// Invoked on dial press (down).
    #[serde(default)]
    pub press: Option<actions::ActionBinding>,
    /// Invoked on dial rotate (delta provided to plugin payload; builtins may ignore).
    #[serde(default)]
    pub rotate: Option<actions::ActionBinding>,
    #[serde(default)]
    pub appearance: Appearance,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TouchStripConfig {
    /// Invoked on touch tap.
    #[serde(default)]
    pub tap: Option<actions::ActionBinding>,
    /// Invoked on touch drag/swipe.
    #[serde(default)]
    pub drag: Option<actions::ActionBinding>,
    #[serde(default)]
    pub appearance: Appearance,
}

#[derive(Debug, Clone)]
pub struct ProfileMeta {
    pub id: ProfileId,
    pub name: String,
    pub path: PathBuf,
    pub key_count: u8,
}

pub fn profiles_dir() -> anyhow::Result<PathBuf> {
    Ok(paths::data_dir()?.join("profiles"))
}

pub fn ensure_profiles_dir() -> anyhow::Result<PathBuf> {
    let dir = profiles_dir()?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn list_profiles() -> anyhow::Result<Vec<ProfileMeta>> {
    let dir = ensure_profiles_dir()?;
    let mut out = vec![];

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(profile) = load_profile(&path) {
            out.push(ProfileMeta {
                id: profile.id,
                name: profile.name,
                key_count: profile.key_count,
                path,
            });
        }
    }

    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

pub fn create_profile(name: &str, key_count: u8) -> anyhow::Result<Profile> {
    let id = new_profile_id();
    let mut profile = Profile {
        version: PROFILE_SCHEMA_VERSION,
        id,
        name: name.to_string(),
        key_count,
        keys: vec![KeyConfig::default(); key_count as usize],
        dials: if key_count == 8 {
            vec![DialConfig::default(); 4]
        } else {
            vec![]
        },
        touch_strip: TouchStripConfig::default(),
    };

    // Give the first profile a minimal default label so UI looks alive.
    if let Some(first) = profile.keys.first_mut() {
        first.label = "Key 0".to_string();
        first.appearance.text = Some(first.label.clone());
    }

    Ok(profile)
}

pub fn profile_path(id: ProfileId) -> anyhow::Result<PathBuf> {
    Ok(ensure_profiles_dir()?.join(format!("{}.json", id.0)))
}

pub fn load_profile(path: &Path) -> anyhow::Result<Profile> {
    let raw = fs::read_to_string(path)?;
    let mut p: Profile = serde_json::from_str(&raw)?;

    if p.version == 0 {
        // Future-proofing: treat missing/zero as v1.
        p.version = 1;
    }

    if p.version == 1 {
        // v1 -> v2: add action bindings (default None).
        for k in &mut p.keys {
            k.action = None;
        }
        p.version = 2;
    }

    if p.version == 2 {
        // v2 -> v3: schema is compatible; builtin actions can now be stored in `action`.
        p.version = 3;
    }

    if p.version == 3 {
        // v3 -> v4: add dials/touch strip + appearance fields (defaults filled by serde).
        p.version = 4;
    }

    if p.version != PROFILE_SCHEMA_VERSION {
        anyhow::bail!("unsupported profile version: {}", p.version);
    }

    // Basic integrity.
    if p.keys.len() != p.key_count as usize {
        p.keys.resize_with(p.key_count as usize, KeyConfig::default);
    }

    // Stream Deck+ convenience: ensure 4 dials if key_count suggests a Plus layout.
    if p.key_count == 8 && p.dials.len() != 4 {
        p.dials.resize_with(4, DialConfig::default);
    }

    Ok(p)
}

pub fn save_profile(profile: &Profile) -> anyhow::Result<()> {
    let path = profile_path(profile.id)?;
    save_profile_to_path(profile, &path)
}

pub fn save_profile_to_path(profile: &Profile, path: &Path) -> anyhow::Result<()> {
    if profile.version != PROFILE_SCHEMA_VERSION {
        anyhow::bail!(
            "refusing to save unsupported profile version: {}",
            profile.version
        );
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(profile)?;

    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(&json)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }

    // Best-effort atomic replace. On Windows, rename over existing may fail; remove first.
    if cfg!(windows) && path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(tmp_path, path)?;
    Ok(())
}

fn new_profile_id() -> ProfileId {
    // Good enough for MVP: time-based unique ID.
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    ProfileId(t.as_nanos() as u64)
}
