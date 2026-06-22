//! Audiobookshelf HTTP API client (reads-first) + config + serde models.
//!
//! Credentials are reused from abs-cli's own config at `~/.abs-cli/config.json`
//! (server + accessToken), so RABSody needs no separate login flow for reads.
//!
//! This module is the API surface and is intentionally built ahead of its
//! consumers during the reads-first migration: models mirror the full ABS
//! response shapes and some client methods land before the commands that use
//! them. Dead-code is allowed here (real dead-code is still caught in `main`).
#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// abs-cli's on-disk config (`~/.abs-cli/config.json`).
#[derive(Debug, Deserialize)]
pub struct AbsConfig {
    pub server: String,
    #[serde(rename = "accessToken")]
    pub access_token: String,
    #[serde(rename = "defaultLibrary")]
    pub default_library: Option<String>,
}

impl AbsConfig {
    /// Load from `~/.abs-cli/config.json`.
    pub fn load() -> Result<Self> {
        let home = dirs::home_dir().context("could not resolve home directory")?;
        let path = home.join(".abs-cli").join("config.json");
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading abs-cli config at {}", path.display()))?;
        let cfg: AbsConfig = serde_json::from_str(&raw).context("parsing abs-cli config.json")?;
        Ok(cfg)
    }
}

/// A page of library items.
#[derive(Debug, Deserialize)]
pub struct ItemsPage {
    pub results: Vec<Item>,
    pub total: u32,
}

#[derive(Debug, Deserialize)]
pub struct Item {
    pub id: String,
    pub media: Media,
}

#[derive(Debug, Deserialize)]
pub struct Media {
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(rename = "numChapters", default)]
    pub num_chapters: u32,
    pub metadata: Metadata,
}

#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub title: Option<String>,
    #[serde(rename = "authorName")]
    pub author_name: Option<String>,
    #[serde(rename = "narratorName")]
    pub narrator_name: Option<String>,
    pub asin: Option<String>,
    pub isbn: Option<String>,
    pub language: Option<String>,
    #[serde(default)]
    pub abridged: bool,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(rename = "seriesName")]
    pub series_name: Option<String>,
    pub subtitle: Option<String>,
}

/// A single result from the provider metadata search.
#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub asin: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub narrator: Option<String>,
    /// Provider duration is in MINUTES (the items endpoint reports seconds).
    pub duration: Option<f64>,
    #[serde(default)]
    pub abridged: bool,
    pub isbn: Option<String>,
    pub language: Option<String>,
}

pub struct Client {
    agent: ureq::Agent,
    server: String,
    token: String,
}

impl Client {
    pub fn new(cfg: &AbsConfig) -> Self {
        Self {
            agent: ureq::Agent::new_with_defaults(),
            server: cfg.server.trim_end_matches('/').to_string(),
            token: cfg.access_token.clone(),
        }
    }

    fn get_json<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let url = format!("{}{}", self.server, path);
        let mut req = self
            .agent
            .get(&url)
            .header("Authorization", &format!("Bearer {}", self.token));
        for (k, v) in query {
            req = req.query(*k, *v);
        }
        let body = req
            .call()
            .with_context(|| format!("GET {url}"))?
            .body_mut()
            .read_json::<T>()
            .with_context(|| format!("decoding response from {url}"))?;
        Ok(body)
    }

    /// `GET /api/me` - identity / auth check.
    pub fn me(&self) -> Result<serde_json::Value> {
        self.get_json("/api/me", &[])
    }

    /// One page of items for a library.
    pub fn items_page(&self, library: &str, page: u32, limit: u32) -> Result<ItemsPage> {
        let path = format!("/api/libraries/{library}/items");
        self.get_json(
            &path,
            &[("limit", &limit.to_string()), ("page", &page.to_string())],
        )
    }

    /// All items for a library, paginated.
    pub fn all_items(&self, library: &str) -> Result<Vec<Item>> {
        const PAGE: u32 = 500;
        let mut out = Vec::new();
        let mut page = 0;
        loop {
            let p = self.items_page(library, page, PAGE)?;
            let got = p.results.len();
            out.extend(p.results);
            if out.len() as u32 >= p.total || got == 0 {
                break;
            }
            page += 1;
        }
        Ok(out)
    }

    /// `GET /api/search/books` - provider metadata search.
    pub fn search_books(
        &self,
        title: &str,
        author: &str,
        provider: &str,
    ) -> Result<Vec<SearchResult>> {
        self.get_json(
            "/api/search/books",
            &[("title", title), ("author", author), ("provider", provider)],
        )
    }
}
