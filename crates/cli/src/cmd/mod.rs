//! One module per verb.

pub mod api_resources;
pub mod apply;
pub mod delete;
pub mod describe;
pub mod diff;
pub mod edit;
pub mod explain;
pub mod get;
pub mod patch;
pub mod sync;
pub mod validate;
pub mod version;
#[cfg(test)]
mod write_tests;

use anyhow::{Result, bail};
use hldr_core::api::ApiResource;
use hldr_core::manifest::Registry;
use serde_json::json;

use crate::client::Client;
use crate::content::{self, Target};
use crate::discovery::Catalog;
use crate::print::Fetched;

/// Flags every writing command takes.
#[derive(Debug, clap::Args)]
pub struct WriteArgs {
    /// Commit message subject, instead of one naming the change
    #[arg(short, long)]
    message: Option<String>,
    /// Commit without asking the server to sync; it catches up on its next poll
    #[arg(long)]
    no_sync: bool,
}

impl WriteArgs {
    pub fn message(&self, change: &str) -> String {
        self.message
            .clone()
            .unwrap_or_else(|| format!("content: {change}"))
    }

    pub fn sync(&self) -> bool {
        !self.no_sync
    }
}

/// What the writing commands share: the API, discovery, and the repository
/// the server reads, opened with the token when there is one. Every GitHub
/// read goes with it: GitHub caches anonymous answers for up to a minute,
/// so an anonymous read right after a write can see the branch before it.
pub struct Writer<'a> {
    pub client: &'a Client,
    pub catalog: Catalog<'a>,
    pub target: Target,
    pub editor: Option<String>,
    registry: Option<Registry>,
}

impl<'a> Writer<'a> {
    pub fn new(
        client: &'a Client,
        catalog: Catalog<'a>,
        api: &str,
        editor: Option<String>,
        token: Option<String>,
    ) -> Result<Self> {
        Ok(Self {
            target: Target::discover(client, api, token)?,
            client,
            catalog,
            editor,
            registry: None,
        })
    }

    /// The server's page types, asked for once per run.
    pub fn registry(&mut self) -> Result<Registry> {
        if self.registry.is_none() {
            self.registry = Some(content::registry(self.client)?);
        }
        Ok(self.registry.clone().unwrap_or_default())
    }

    /// The server's resource types, to name what a sync changed.
    pub fn resources(&mut self) -> Result<Vec<ApiResource>> {
        Ok(self.catalog.discovery()?.resources.clone())
    }

    pub fn resource(&mut self, kind: &str, verb: &str) -> Result<ApiResource> {
        let resource = self.catalog.resolve(kind)?;
        if !resource.verbs.iter().any(|v| v == verb) {
            bail!("{} does not support {verb}", resource.name);
        }
        Ok(resource)
    }

    /// Fails before any work when there is nothing to write with.
    pub fn authorize(&self) -> Result<()> {
        if !self.target.github.has_token() {
            bail!(
                "writing needs a GitHub token: set HLDR_GITHUB_TOKEN, or `content.token_command` \
                 in the config file"
            );
        }
        Ok(())
    }
}

/// `TYPE`, `TYPE NAME...` or `TYPE/NAME...`, grouped by type in the order
/// the types first appear.
pub fn requests(targets: &[String]) -> Result<Vec<(String, Vec<String>)>> {
    let Some(first) = targets.first() else {
        bail!("name a resource type, such as `projects`; `hldr api-resources` lists them");
    };
    if !first.contains('/') {
        if let Some(slashed) = targets[1..].iter().find(|t| t.contains('/')) {
            bail!("{slashed:?}: use TYPE NAME... or TYPE/NAME..., not both");
        }
        return Ok(vec![(first.clone(), targets[1..].to_vec())]);
    }
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for target in targets {
        let Some((kind, name)) = target.split_once('/') else {
            bail!("{target:?}: expected TYPE/NAME like the arguments before it");
        };
        match groups.iter_mut().find(|(k, _)| k == kind) {
            Some((_, names)) => names.push(name.to_owned()),
            None => groups.push((kind.to_owned(), vec![name.to_owned()])),
        }
    }
    Ok(groups)
}

/// Fetches what `targets` names. A missing name is reported on stderr and
/// makes the command fail at the end, as kubectl does, without hiding the
/// resources that were found.
pub fn fetch(
    client: &Client,
    catalog: &mut Catalog<'_>,
    targets: &[String],
    verb: &str,
) -> Result<(Vec<Fetched>, bool)> {
    let mut fetched = Vec::new();
    let mut complete = true;
    for (kind, names) in requests(targets)? {
        let resource = catalog.resolve(&kind)?;
        if !resource.verbs.iter().any(|v| v == verb) {
            bail!("{} does not support {verb}", resource.name);
        }
        let collection = format!("/api/v1/{}", resource.name);
        if names.is_empty() {
            let value = client.get(&collection)?;
            fetched.push(Fetched { resource, value });
            continue;
        }
        if resource.singleton {
            bail!(
                "{0} is a singleton: `hldr {verb} {0}` takes no name",
                resource.singular
            );
        }
        let mut items = Vec::new();
        for name in &names {
            if !hldr_core::manifest::is_valid_name(name) {
                bail!("{name:?} is not a resource name: names match [a-z0-9][a-z0-9-]*");
            }
            match client.get_optional(&format!("{collection}/{name}"))? {
                Some(item) => items.push(item),
                None => {
                    eprintln!("error: {} {name:?} not found", resource.singular);
                    complete = false;
                }
            }
        }
        let value = match items.len() {
            0 => continue,
            1 => items.remove(0),
            _ => json!({ "kind": format!("{}List", resource.kind), "items": items }),
        };
        fetched.push(Fetched { resource, value });
    }
    Ok((fetched, complete))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn groups_targets_by_type() {
        assert_eq!(
            requests(&args(&["projects"])).unwrap(),
            [("projects".to_owned(), vec![])]
        );
        assert_eq!(
            requests(&args(&["p", "atlas", "hldr"])).unwrap(),
            [("p".to_owned(), args(&["atlas", "hldr"]))]
        );
        assert_eq!(
            requests(&args(&["p/atlas", "theme/nord", "p/hldr"])).unwrap(),
            [
                ("p".to_owned(), args(&["atlas", "hldr"])),
                ("theme".to_owned(), args(&["nord"]))
            ]
        );
        assert!(requests(&args(&["p/atlas", "hldr"])).is_err());
        assert!(requests(&args(&["p", "theme/nord"])).is_err());
        assert!(requests(&[]).is_err());
    }
}
