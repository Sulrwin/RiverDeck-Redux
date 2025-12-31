//! OpenAction Marketplace client (fetch plugin index for discovery).
//!
//! The repo currently supports local plugin installs only. This module enables
//! reading an HTTP-hosted marketplace "index" so UIs can display available plugins.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum DownloadSpec {
    Url(String),
    Platforms(DownloadPlatforms),
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DownloadPlatforms {
    #[serde(default)]
    linux: Option<String>,
    #[serde(default)]
    windows: Option<String>,
    #[serde(default)]
    macos: Option<String>,
}

fn pick_platform_download(spec: &DownloadSpec) -> Option<String> {
    match spec {
        DownloadSpec::Url(u) => {
            let u = u.trim();
            if u.is_empty() { None } else { Some(u.to_string()) }
        }
        DownloadSpec::Platforms(p) => {
            let pick = if cfg!(windows) {
                p.windows.as_deref()
            } else if cfg!(target_os = "macos") {
                p.macos.as_deref()
            } else {
                p.linux.as_deref()
            };
            pick.map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| s.to_string())
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplacePlugin {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    /// Optional source repository URL (often GitHub).
    #[serde(default)]
    pub repository: Option<String>,
    /// Optional URL pointing to an icon for display in UIs.
    #[serde(default)]
    #[serde(alias = "iconUrl")]
    pub icon_url: Option<String>,
    /// Optional URL for downloading/installing the plugin artifact.
    ///
    /// Note: the install flow is intentionally out of scope for MVP.
    #[serde(default)]
    #[serde(alias = "downloadUrl")]
    pub download_url: Option<String>,
    /// Optional alternative download shape (string or platform-specific map).
    #[serde(default)]
    pub downloads: Option<DownloadSpec>,
    /// Optional screenshot/image URLs for marketplace details.
    #[serde(default, alias = "screenshots", alias = "images")]
    pub images: Vec<String>,
}

/// The Rivul/OpenAction catalogue shape is a map keyed by plugin ID:
/// `{ "<plugin_id>": { "name": "...", "author": "...", ... } }`
#[derive(Debug, Clone, Deserialize)]
struct CatalogueEntry {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    #[serde(alias = "iconUrl")]
    pub icon_url: Option<String>,
    #[serde(default)]
    #[serde(alias = "downloadUrl")]
    pub download_url: Option<String>,
    #[serde(default)]
    pub downloads: Option<DownloadSpec>,
    #[serde(default, alias = "screenshots", alias = "images")]
    pub images: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum MarketplaceResponse {
    List(Vec<MarketplacePlugin>),
    Wrapped { plugins: Vec<MarketplacePlugin> },
    Catalogue(BTreeMap<String, CatalogueEntry>),
}

/// Fetches a marketplace index from the provided URL.
///
/// Supported JSON shapes:
/// - `[{...}, {...}]`
/// - `{ "plugins": [{...}, {...}] }`
/// - `{ "<plugin_id>": { "name": "...", ... }, ... }` (OpenAction catalogue.json)
pub async fn fetch_plugins(index_url: &str) -> anyhow::Result<Vec<MarketplacePlugin>> {
    let bytes = fetch_bytes(index_url).await?;
    let parsed: MarketplaceResponse = serde_json::from_slice(&bytes).map_err(|e| {
        // Include a small, safe preview to help diagnose wrong endpoints (HTML, etc.).
        let preview = String::from_utf8_lossy(&bytes[..bytes.len().min(240)]);
        anyhow::anyhow!("failed to parse marketplace JSON: {e}. body preview: {preview}")
    })?;

    let mut plugins: Vec<MarketplacePlugin> = match parsed {
        MarketplaceResponse::List(list) => list,
        MarketplaceResponse::Wrapped { plugins } => plugins,
        MarketplaceResponse::Catalogue(map) => map
            .into_iter()
            .map(|(id, e)| MarketplacePlugin {
                id,
                name: e.name,
                version: e.version,
                description: e.description.unwrap_or_default(),
                author: e.author,
                homepage: e.homepage,
                repository: e.repository,
                icon_url: e.icon_url,
                download_url: e
                    .download_url
                    .or_else(|| e.downloads.as_ref().and_then(pick_platform_download)),
                downloads: e.downloads,
                images: e.images,
            })
            .collect(),
    };

    // Normalize: if the marketplace provides an alternate download shape, prefer it.
    for p in &mut plugins {
        if p.download_url.as_deref().map(|s| s.trim()).unwrap_or("").is_empty() {
            p.download_url = p.downloads.as_ref().and_then(pick_platform_download);
        }
    }

    plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(plugins)
}

/// Fetch raw bytes from a marketplace URL.
///
/// We set `Accept-Encoding: identity` to keep things predictable (plain bodies),
/// especially when `reqwest` is built with a reduced feature set.
pub async fn fetch_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .user_agent("RiverDeck-Redux/0.1 (OpenAction Marketplace)")
        .build()?;

    let resp = client
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await?
        .error_for_status()?;

    Ok(resp.bytes().await?.to_vec())
}


