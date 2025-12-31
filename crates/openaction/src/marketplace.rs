//! OpenAction Marketplace client (fetch plugin index for discovery).
//!
//! The repo currently supports local plugin installs only. This module enables
//! reading an HTTP-hosted marketplace "index" so UIs can display available plugins.

use serde::Deserialize;
use std::collections::BTreeMap;

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
    /// Optional URL for downloading/installing the plugin artifact.
    ///
    /// Note: the install flow is intentionally out of scope for MVP.
    #[serde(default)]
    pub download_url: Option<String>,
}

/// The Rivul/OpenAction catalogue shape is a map keyed by plugin ID:
/// `{ "<plugin_id>": { "name": "...", "author": "...", ... } }`
#[derive(Debug, Clone, Deserialize)]
struct CatalogueEntry {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
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
    let client = reqwest::Client::builder()
        .user_agent("RiverDeck-Redux/0.1 (OpenAction Marketplace)")
        .build()?;

    let resp = client.get(index_url).send().await?.error_for_status()?;
    let parsed: MarketplaceResponse = resp.json().await?;

    let mut plugins = match parsed {
        MarketplaceResponse::List(list) => list,
        MarketplaceResponse::Wrapped { plugins } => plugins,
        MarketplaceResponse::Catalogue(map) => map
            .into_iter()
            .map(|(id, e)| MarketplacePlugin {
                id,
                name: e.name,
                version: e.version,
                description: e.description,
                author: e.author,
                homepage: e.homepage,
                download_url: e.download_url,
            })
            .collect(),
    };

    plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(plugins)
}


