//! Writing content: finding where the server reads it, turning files into
//! changes, committing them, and having the server sync the new commit.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hldr_core::api::ApiResource;
use hldr_core::manifest;
use serde_json::{Value, json};

use crate::client::Client;
use crate::discovery::Catalog;
use crate::github::{Change, GitHub};

/// The repository and branch the server syncs from, and so where the CLI
/// writes. Asking the server keeps the two from ever disagreeing.
pub struct Target {
    pub github: GitHub,
    pub branch: String,
}

impl Target {
    pub fn discover(client: &Client, api: &str, token: Option<String>) -> Result<Self> {
        let status = client.get("/api/v1/sync")?;
        let source = status["source"].as_str().unwrap_or("an unknown source");
        let (Some(repo), Some(branch)) = (status["repository"].as_str(), status["branch"].as_str())
        else {
            bail!(
                "the server reads content from {source}, not from a GitHub repository: there is nothing to write to"
            );
        };
        let valid_branch = !branch.is_empty()
            && !branch.contains("..")
            && branch
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'));
        if !valid_branch {
            bail!("the server reports an invalid branch {branch:?}");
        }
        Ok(Self {
            github: GitHub::new(api, repo, token)?,
            branch: branch.to_owned(),
        })
    }

    pub fn head(&self) -> Result<String> {
        self.github.head(&self.branch)
    }
}

/// Where a resource's file lives, from the server's discovery.
pub fn path_for(resource: &ApiResource, name: Option<&str>) -> Result<String> {
    match (resource.singleton, name) {
        (true, None) => Ok(resource.source.path.clone()),
        (true, Some(_)) => bail!("{0} is a singleton: it takes no name", resource.singular),
        (false, None) => bail!("{} needs a name", resource.singular),
        (false, Some(name)) if manifest::is_valid_name(name) => {
            Ok(resource.source.path.replace("{name}", name))
        }
        (false, Some(name)) => {
            bail!("{name:?} is not a resource name: names match [a-z0-9][a-z0-9-]*")
        }
    }
}

/// Checks a file with the parser the server indexes with. A kind this CLI
/// does not know yet is left to the server, which checks it on sync.
pub fn validate(path: &str, text: &str) -> Result<()> {
    if manifest::locate(Path::new(path))?.is_none() {
        eprintln!("warning: this hldr cannot check {path}; the server will when it syncs");
        return Ok(());
    }
    manifest::parse(Path::new(path), text.as_bytes())?;
    Ok(())
}

/// A file named on the command line, resolved to its place in the content.
pub struct LocalFile {
    /// As the user named it, for messages.
    pub origin: String,
    pub resource: ApiResource,
    pub name: Option<String>,
    /// Path under the content root.
    pub path: String,
    pub text: String,
}

impl LocalFile {
    /// `project/atlas`, or `site`.
    pub fn label(&self) -> String {
        match &self.name {
            Some(name) => format!("{}/{name}", self.resource.singular),
            None => self.resource.singular.clone(),
        }
    }
}

/// Reads `-f` arguments. A file goes where its declared kind and its file
/// name say; a directory is read as a content tree, each file at its path
/// under it, skipping what the layout does not hold; `-` reads a singleton
/// from stdin.
pub fn collect(catalog: &mut Catalog<'_>, args: &[PathBuf], verb: &str) -> Result<Vec<LocalFile>> {
    let mut files = Vec::new();
    for arg in args {
        if arg.as_os_str() == "-" {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .context("reading stdin")?;
            files.push(resolve(catalog, "stdin", None, text, verb)?);
        } else if arg.is_dir() {
            for (rel, full) in walk(arg)? {
                let Some(location) = manifest::locate(&rel).ok().flatten() else {
                    eprintln!("skipping {}: not a content path", full.display());
                    continue;
                };
                let text = read(&full)?;
                let file = resolve(
                    catalog,
                    &full.display().to_string(),
                    location.name,
                    text,
                    verb,
                )?;
                if file.path != rel.to_string_lossy() {
                    bail!(
                        "{}: declares {}, which lives at {}",
                        full.display(),
                        file.label(),
                        file.path
                    );
                }
                files.push(file);
            }
        } else {
            let name = arg
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned);
            files.push(resolve(
                catalog,
                &arg.display().to_string(),
                name,
                read(arg)?,
                verb,
            )?);
        }
    }
    let mut seen = std::collections::HashSet::new();
    for file in &files {
        if !seen.insert(&file.path) {
            bail!("{} is given twice", file.path);
        }
    }
    if files.is_empty() {
        bail!("no content files to {verb}");
    }
    Ok(files)
}

fn resolve(
    catalog: &mut Catalog<'_>,
    origin: &str,
    name: Option<String>,
    text: String,
    verb: &str,
) -> Result<LocalFile> {
    let kind = manifest::declared_kind(&text)
        .with_context(|| format!("{origin}: no `kind:` to say what it is"))?;
    let resource = catalog.resolve(&kind).with_context(|| origin.to_owned())?;
    if !resource.verbs.iter().any(|v| v == verb) {
        bail!("{origin}: {} does not support {verb}", resource.name);
    }
    let name = if resource.singleton {
        None
    } else if origin == "stdin" {
        bail!(
            "{} takes its name from its file name, so it cannot come from stdin",
            resource.singular
        );
    } else {
        name
    };
    let path = path_for(&resource, name.as_deref()).with_context(|| origin.to_owned())?;
    validate(&path, &text).with_context(|| origin.to_owned())?;
    Ok(LocalFile {
        origin: origin.to_owned(),
        resource,
        name,
        path,
        text,
    })
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// Files under `root`, as (path relative to root, full path), skipping
/// hidden entries such as `.git`.
fn walk(root: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in
            std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?
        {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push((rel.to_owned(), path.clone()));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Commits `changes` and, unless told not to, has the server sync that exact
/// commit before returning.
pub struct Publish<'a> {
    pub client: &'a Client,
    pub target: &'a Target,
    pub parent: &'a str,
    pub message: String,
    pub sync: bool,
}

impl Publish<'_> {
    pub fn run(self, out: &mut dyn Write, changes: &[Change]) -> Result<()> {
        let message = format!(
            "{}\n\nHldr-Client: hldr {}",
            self.message,
            hldr_core::VERSION
        );
        let commit =
            self.target
                .github
                .commit(&self.target.branch, self.parent, changes, &message)?;
        writeln!(
            out,
            "committed {} to {}@{}",
            short(&commit),
            self.target.github.repo(),
            self.target.branch
        )?;
        if !self.sync {
            writeln!(
                out,
                "not synced: the server picks it up on its next poll, or run `hldr sync`"
            )?;
            return Ok(());
        }
        let status = self
            .client
            .post("/api/v1/sync", &json!({ "revision": commit }))
            .with_context(|| {
                format!("committed {}, but the site did not update", short(&commit))
            })?;
        writeln!(out, "synced: {}", summary(&status))?;
        Ok(())
    }
}

/// What a sync changed, in a line.
pub fn summary(status: &Value) -> String {
    let revision = status["revision"].as_str().map_or("<none>", short);
    let report = &status["report"];
    if !report.is_object() {
        return format!("{revision}, already current");
    }
    let count = |key: &str| report[key].as_u64().unwrap_or(0);
    let plural = |n: u64, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
    let mut parts = Vec::new();
    if report["site_updated"] == true {
        parts.push("site".to_owned());
    }
    if report["profile_updated"] == true {
        parts.push("profile".to_owned());
    }
    for (key, what) in [
        ("projects_upserted", "project"),
        ("themes_upserted", "theme"),
    ] {
        if count(key) > 0 {
            parts.push(plural(count(key), what));
        }
    }
    let mut removed = Vec::new();
    for (key, what) in [("projects_deleted", "project"), ("themes_deleted", "theme")] {
        if count(key) > 0 {
            removed.push(plural(count(key), what));
        }
    }
    let mut line = revision.to_owned();
    if !parts.is_empty() {
        line.push_str(&format!(", updated {}", parts.join(", ")));
    }
    if !removed.is_empty() {
        line.push_str(&format!(", removed {}", removed.join(", ")));
    }
    if parts.is_empty() && removed.is_empty() {
        line.push_str(", nothing changed");
    }
    line
}

pub fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_a_sync() {
        let status = json!({"revision": "0123456789abcdef", "report": {
            "site_updated": false, "profile_updated": true,
            "projects_upserted": 2, "projects_skipped": 3, "projects_deleted": 1,
            "themes_upserted": 1, "themes_skipped": 32, "themes_deleted": 0,
        }});
        assert_eq!(
            summary(&status),
            "0123456789ab, updated profile, 2 projects, 1 theme, removed 1 project"
        );
        assert_eq!(summary(&json!({"revision": "abc"})), "abc, already current");
    }
}
