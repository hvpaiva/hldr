//! One module per verb.

pub mod api_resources;
pub mod describe;
pub mod explain;
pub mod get;
pub mod version;

use anyhow::{Result, bail};
use serde_json::json;

use crate::client::Client;
use crate::discovery::Catalog;
use crate::print::Fetched;

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
