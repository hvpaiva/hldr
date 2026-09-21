//! What resource types the server has, cached like kubectl's discovery.
//!
//! The catalog and the schemas come from the server, so a kind added there
//! works here without a new CLI. The cache lives per server under
//! `$XDG_CACHE_HOME/hldr/` and is refreshed after six hours, when the server
//! runs another version than the one that filled it, or at once when a type
//! is not found in it.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use hldr_core::api::{ApiResource, List};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::client::Client;

const TTL: Duration = Duration::from_secs(6 * 3600);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovery {
    fetched_at: u64,
    /// Version of the server that answered. An upgrade can change the kinds
    /// and their schemas, so a cache from another version is stale.
    #[serde(default)]
    server_version: Option<String>,
    pub resources: Vec<ApiResource>,
    pub schemas: Map<String, Value>,
}

impl Discovery {
    fn fetch(client: &Client, now: u64) -> Result<Self> {
        let resources: List<ApiResource> =
            serde_json::from_value(client.get("/api/v1/api-resources")?)
                .context("the server sent an invalid resource list")?;
        let Value::Object(schemas) = client.get("/api/v1/schema")? else {
            bail!("the server sent an invalid schema document");
        };
        Ok(Self {
            fetched_at: now,
            server_version: server_version(client)?,
            resources: resources.items,
            schemas,
        })
    }

    /// Finds a type by plural, singular, short name or kind, ignoring case.
    pub fn find(&self, name: &str) -> Option<&ApiResource> {
        let name = name.to_ascii_lowercase();
        self.resources.iter().find(|resource| {
            resource.name == name
                || resource.singular == name
                || resource.kind.to_ascii_lowercase() == name
                || resource.short_names.contains(&name)
        })
    }
}

/// Discovery for one server, loaded from the cache when fresh.
pub struct Catalog<'a> {
    client: &'a Client,
    cache: Option<PathBuf>,
    discovery: Option<Discovery>,
    fresh: bool,
}

impl<'a> Catalog<'a> {
    pub fn new(client: &'a Client, cache_dir: Option<&Path>) -> Self {
        let cache = cache_dir.map(|dir| dir.join(host_dir(client.base())).join("discovery.json"));
        Self {
            client,
            cache,
            discovery: None,
            fresh: false,
        }
    }

    pub fn discovery(&mut self) -> Result<&Discovery> {
        let loaded = match self.discovery.take() {
            Some(discovery) => discovery,
            None => match self
                .read_cache()
                .filter(|d| now().saturating_sub(d.fetched_at) < TTL.as_secs())
            {
                Some(cached) if cached.server_version == server_version(self.client)? => cached,
                _ => self.fetch()?,
            },
        };
        Ok(self.discovery.insert(loaded))
    }

    /// Asks the server, at most once per run.
    pub fn refresh(&mut self) -> Result<&Discovery> {
        if self.fresh {
            return self.discovery();
        }
        let fetched = self.fetch()?;
        Ok(self.discovery.insert(fetched))
    }

    fn fetch(&mut self) -> Result<Discovery> {
        let discovery = Discovery::fetch(self.client, now())?;
        self.write_cache(&discovery);
        self.fresh = true;
        Ok(discovery)
    }

    /// Resolves a type, refreshing a stale cache before giving up.
    pub fn resolve(&mut self, name: &str) -> Result<ApiResource> {
        if let Some(found) = self.discovery()?.find(name) {
            return Ok(found.clone());
        }
        match self.refresh()?.find(name) {
            Some(found) => Ok(found.clone()),
            None => bail!("the server doesn't have a resource type {name:?}"),
        }
    }

    fn read_cache(&self) -> Option<Discovery> {
        let text = std::fs::read_to_string(self.cache.as_ref()?).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// The cache is an optimization: failing to write it is not an error.
    fn write_cache(&self, discovery: &Discovery) {
        let Some(path) = &self.cache else { return };
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                let json = serde_json::to_vec(discovery).map_err(std::io::Error::other)?;
                let tmp = path.with_extension("json.tmp");
                std::fs::write(&tmp, json)?;
                std::fs::rename(&tmp, path)
            });
        if let Err(err) = written {
            eprintln!("warning: discovery cache {}: {err}", path.display());
        }
    }
}

/// What `/healthz` reports, which never touches the server's database.
fn server_version(client: &Client) -> Result<Option<String>> {
    Ok(client.get("/healthz")?["version"]
        .as_str()
        .map(str::to_owned))
}

/// One cache directory per server, named after its URL.
fn host_dir(base: &str) -> String {
    base.split_once("://")
        .map_or(base, |(_, rest)| rest)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stub::{self, Stub};

    fn resource(name: &str, singular: &str, short: &[&str], kind: &str) -> ApiResource {
        serde_json::from_value(serde_json::json!({
            "name": name, "singular": singular, "short_names": short, "kind": kind,
            "singleton": false, "verbs": ["get"],
            "source": {"path": format!("{name}/{{name}}.md"), "format": "markdown"},
            "columns": [],
        }))
        .unwrap()
    }

    #[test]
    fn finds_by_any_name() {
        let discovery = Discovery {
            fetched_at: 0,
            server_version: None,
            resources: vec![resource("projects", "project", &["proj", "p"], "Project")],
            schemas: Map::new(),
        };
        for name in ["projects", "project", "p", "proj", "Project", "PROJECTS"] {
            assert_eq!(discovery.find(name).unwrap().kind, "Project", "{name}");
        }
        assert!(discovery.find("posts").is_none());
    }

    #[test]
    fn host_dirs_are_path_safe() {
        assert_eq!(
            host_dir("https://apollo.example.ts.net:8443"),
            "apollo.example.ts.net_8443"
        );
        assert_eq!(host_dir("http://127.0.0.1:8081"), "127.0.0.1_8081");
        assert_eq!(host_dir("https://x/../../etc"), "x_.._.._etc");
    }

    #[test]
    fn caches_per_server_and_refreshes_on_a_miss() {
        let stub = Stub::default();
        let client = Client::new(stub.serve());
        let cache = tempfile::tempdir().unwrap();

        Catalog::new(&client, Some(cache.path()))
            .resolve("p")
            .unwrap_err();
        assert_eq!(
            stub.calls(),
            1,
            "a run that fetched does not fetch again on a miss"
        );
        Catalog::new(&client, Some(cache.path()))
            .resolve("project")
            .unwrap();
        assert_eq!(stub.calls(), 1, "a fresh cache answers without the server");

        stub.extra
            .lock()
            .unwrap()
            .push(stub::resource("themes", "theme", "Theme"));
        let mut catalog = Catalog::new(&client, Some(cache.path()));
        assert_eq!(catalog.resolve("theme").unwrap().kind, "Theme");
        assert_eq!(stub.calls(), 2, "a kind the cache lacks is fetched");
        catalog.resolve("posts").unwrap_err();
        assert_eq!(stub.calls(), 2, "one refresh per run");
    }

    #[test]
    fn a_server_upgrade_invalidates_the_cache() {
        let stub = Stub::default();
        stub.set_version("3.4.0");
        let client = Client::new(stub.serve());
        let cache = tempfile::tempdir().unwrap();

        Catalog::new(&client, Some(cache.path()))
            .discovery()
            .unwrap();
        Catalog::new(&client, Some(cache.path()))
            .discovery()
            .unwrap();
        assert_eq!(stub.calls(), 1, "same version, cached");

        stub.set_version("3.5.0");
        Catalog::new(&client, Some(cache.path()))
            .discovery()
            .unwrap();
        assert_eq!(stub.calls(), 2, "another version, fetched again");
        Catalog::new(&client, Some(cache.path()))
            .discovery()
            .unwrap();
        assert_eq!(stub.calls(), 2, "and cached for it");
    }
}
