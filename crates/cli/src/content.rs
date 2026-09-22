//! Writing content: finding where the server reads it, turning files into
//! changes, committing them, and having the server sync the new commit.
//!
//! A page travels as two files, its manifest and the markdown it names:
//! whatever writes one takes the other along.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hldr_core::api::ApiResource;
use hldr_core::manifest::{self, Location, Manifest, Registry};
use hldr_core::spec::{CollectionSpec, PageKindSpec};
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

/// The page types the server serves: they decide what the pages of each
/// collection declare, so the CLI checks a page as the server will.
pub fn registry(client: &Client) -> Result<Registry> {
    let kinds = client.get("/api/v1/pagekinds")?;
    let collections = client.get("/api/v1/collections")?;
    let items = |list: &Value| list["items"].as_array().cloned().unwrap_or_default();
    let kinds: Vec<PageKindSpec> = items(&kinds)
        .into_iter()
        .map(|item| serde_json::from_value(item["spec"].clone()))
        .collect::<Result<_, _>>()
        .context("the server sent an invalid page type")?;
    let collections: Vec<(String, CollectionSpec)> = items(&collections)
        .into_iter()
        .map(|item| {
            let name = item["metadata"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            serde_json::from_value(item["spec"].clone()).map(|spec| (name, spec))
        })
        .collect::<Result<_, _>>()
        .context("the server sent an invalid collection")?;
    Ok(Registry::new(kinds, collections))
}

/// Where a resource's manifest lives, from the server's discovery.
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

/// Checks a manifest with the parser the server indexes with.
pub fn validate(registry: &Registry, path: &str, text: &str) -> Result<Manifest> {
    Ok(manifest::parse(Path::new(path), text.as_bytes(), registry)?)
}

/// Checks a page's markdown: no frontmatter, and directives that parse and
/// fit the page's type.
pub fn validate_content(registry: &Registry, manifest: &Manifest, text: &str) -> Result<()> {
    if text.starts_with("---\n") || text.starts_with("---\r\n") {
        bail!(
            "content has no frontmatter: its fields belong in {}",
            manifest.path()
        );
    }
    registry
        .check_content(manifest.kind(), text)
        .map_err(|problems| anyhow::anyhow!(problems.join("\n")))
}

/// The markdown a manifest names as its content file, read without checking
/// the rest, so it works on a manifest that no longer validates.
pub fn named_content(path: &str, text: &str) -> Option<String> {
    let document: Value = serde_saphyr::from_str(text).ok()?;
    let file = document["spec"]["content"]["file"].as_str()?;
    let dir = Path::new(path).parent()?;
    Some(dir.join(file).to_string_lossy().into_owned())
}

/// A manifest named on the command line, resolved to its place in the
/// content, with the markdown it names when that came along.
pub struct LocalFile {
    /// As the user named it, for messages.
    pub origin: String,
    pub resource: ApiResource,
    pub name: Option<String>,
    /// Path under the content root.
    pub path: String,
    pub text: String,
    pub manifest: Manifest,
    pub content: Option<LocalContent>,
}

/// A page's markdown, read beside its manifest.
pub struct LocalContent {
    pub origin: String,
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

    /// Where the markdown this manifest names lives, if it names a file.
    pub fn content_path(&self) -> Option<String> {
        match &self.manifest {
            Manifest::Page(page) => page.content_path(),
            _ => None,
        }
    }
}

/// Reads `-f` arguments. A manifest goes where its declared kind and its
/// file name say, and takes along the markdown it names from beside it; a
/// directory is read as a content tree, each file at its path under it,
/// skipping what the layout does not hold; `-` reads a singleton from stdin.
pub fn collect(
    catalog: &mut Catalog<'_>,
    registry: &Registry,
    args: &[PathBuf],
    verb: &str,
) -> Result<Vec<LocalFile>> {
    let mut files = Vec::new();
    for arg in args {
        if arg.as_os_str() == "-" {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .context("reading stdin")?;
            files.push(resolve(catalog, registry, "stdin", None, text, verb)?);
        } else if arg.is_dir() {
            files.extend(collect_tree(catalog, registry, arg, verb)?);
        } else {
            if arg.extension().is_some_and(|ext| ext == "md") {
                bail!(
                    "{}: a page's markdown goes with its manifest; give the .yaml that names it",
                    arg.display()
                );
            }
            let name = arg
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned);
            let mut file = resolve(
                catalog,
                registry,
                &arg.display().to_string(),
                name,
                read(arg)?,
                verb,
            )?;
            if let Some(content_path) = file.content_path() {
                let beside =
                    arg.with_file_name(Path::new(&content_path).file_name().unwrap_or_default());
                if beside.is_file() {
                    let text = read(&beside)?;
                    validate_content(registry, &file.manifest, &text)
                        .with_context(|| beside.display().to_string())?;
                    file.content = Some(LocalContent {
                        origin: beside.display().to_string(),
                        path: content_path,
                        text,
                    });
                }
            }
            files.push(file);
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

fn collect_tree(
    catalog: &mut Catalog<'_>,
    registry: &Registry,
    root: &Path,
    verb: &str,
) -> Result<Vec<LocalFile>> {
    let mut files = Vec::new();
    let mut markdown = BTreeMap::new();
    for rel in hldr_core::tree::paths(root)? {
        let full = root.join(&rel);
        let location = manifest::locate(&rel).ok().flatten();
        let name = match location {
            Some(Location::Content { .. }) => {
                markdown.insert(rel.to_string_lossy().into_owned(), full);
                continue;
            }
            Some(Location::Manifest { name, .. }) => name,
            Some(Location::Member { name, .. }) => Some(name),
            None => {
                eprintln!("skipping {}: not a content path", full.display());
                continue;
            }
        };
        let file = resolve(
            catalog,
            registry,
            &full.display().to_string(),
            name,
            read(&full)?,
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
    for file in &mut files {
        let Some(content_path) = file.content_path() else {
            continue;
        };
        if let Some(full) = markdown.remove(&content_path) {
            let text = read(&full)?;
            validate_content(registry, &file.manifest, &text)
                .with_context(|| full.display().to_string())?;
            file.content = Some(LocalContent {
                origin: full.display().to_string(),
                path: content_path,
                text,
            });
        }
    }
    if let Some((path, _)) = markdown.into_iter().next() {
        bail!(
            "{}: no page names it as its content; set it as a manifest's spec.content.file, or remove it",
            root.join(path).display()
        );
    }
    Ok(files)
}

fn resolve(
    catalog: &mut Catalog<'_>,
    registry: &Registry,
    origin: &str,
    name: Option<String>,
    text: String,
    verb: &str,
) -> Result<LocalFile> {
    let text = without_server_fields(text);
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
    let manifest = validate(registry, &path, &text).with_context(|| origin.to_owned())?;
    Ok(LocalFile {
        origin: origin.to_owned(),
        resource,
        name,
        path,
        text,
        manifest,
        content: None,
    })
}

/// A file as `hldr get -o yaml` printed it carries what the server keeps:
/// `status` and the timestamps in `metadata`. Apply leaves those out, and
/// leaves any other file exactly as written.
fn without_server_fields(text: String) -> String {
    let Ok(Value::Object(mut document)) = serde_saphyr::from_str::<Value>(&text) else {
        return text;
    };
    let mut changed = document.remove("status").is_some();
    if let Some(Value::Object(metadata)) = document.get_mut("metadata") {
        for key in ["created_at", "updated_at"] {
            changed |= metadata.remove(key).is_some();
        }
    }
    if !changed {
        return text;
    }
    serde_saphyr::to_string(&Value::Object(document)).unwrap_or(text)
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// Commits `changes` and, unless told not to, has the server sync that exact
/// commit before returning.
pub struct Publish<'a> {
    pub client: &'a Client,
    pub target: &'a Target,
    /// The server's resource types, to name what a sync changed.
    pub resources: &'a [ApiResource],
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
        writeln!(out, "synced: {}", summary(&status, self.resources))?;
        Ok(())
    }
}

/// What a sync changed, in a line: singletons by name, then the other kinds
/// counted, in the words discovery gives them.
pub fn summary(status: &Value, resources: &[ApiResource]) -> String {
    let revision = status["revision"].as_str().map_or("<none>", short);
    let Some(kinds) = status["report"]["kinds"].as_object() else {
        return format!("{revision}, already current");
    };
    let noun = |kind: &str, n: u64| match resources.iter().find(|resource| resource.kind == kind) {
        Some(resource) if resource.singleton => resource.singular.clone(),
        Some(resource) if n == 1 => format!("1 {}", resource.singular),
        Some(resource) => format!("{n} {}", resource.name),
        None => format!("{n} {}", kind.to_lowercase()),
    };
    let singleton = |kind: &str| {
        resources
            .iter()
            .any(|resource| resource.kind == kind && resource.singleton)
    };
    let mut order: Vec<&String> = kinds.keys().collect();
    order.sort_by_key(|kind| (!singleton(kind), kind.as_str()));
    let count = |kind: &str, key: &str| kinds[kind][key].as_u64().unwrap_or(0);
    let updated: Vec<String> = order
        .iter()
        .filter(|kind| count(kind, "upserted") > 0)
        .map(|kind| noun(kind, count(kind, "upserted")))
        .collect();
    let removed: Vec<String> = order
        .iter()
        .filter(|kind| count(kind, "deleted") > 0)
        .map(|kind| noun(kind, count(kind, "deleted")))
        .collect();
    let mut line = revision.to_owned();
    if !updated.is_empty() {
        line.push_str(&format!(", updated {}", updated.join(", ")));
    }
    if !removed.is_empty() {
        line.push_str(&format!(", removed {}", removed.join(", ")));
    }
    if updated.is_empty() && removed.is_empty() {
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

    fn resource(name: &str, singular: &str, kind: &str, singleton: bool) -> ApiResource {
        serde_json::from_value(json!({
            "name": name, "singular": singular, "short_names": [], "kind": kind,
            "singleton": singleton, "verbs": ["get"],
            "source": {"path": format!("{name}/{{name}}.yaml")}, "columns": [],
        }))
        .unwrap()
    }

    #[test]
    fn summarizes_a_sync() {
        let resources = [
            resource("profile", "profile", "Profile", true),
            resource("projects", "project", "Project", false),
            resource("themes", "theme", "Theme", false),
        ];
        let status = json!({"revision": "0123456789abcdef", "report": {"kinds": {
            "Profile": {"upserted": 1, "skipped": 0, "deleted": 0},
            "Project": {"upserted": 2, "skipped": 3, "deleted": 1},
            "Theme": {"upserted": 1, "skipped": 32, "deleted": 0},
            "Widget": {"upserted": 3},
        }}});
        assert_eq!(
            summary(&status, &resources),
            "0123456789ab, updated profile, 2 projects, 1 theme, 3 widget, removed 1 project"
        );
        assert_eq!(
            summary(&json!({"revision": "abc"}), &resources),
            "abc, already current"
        );
    }

    #[test]
    fn apply_leaves_out_what_the_server_keeps() {
        let printed = "kind: Theme\nmetadata:\n  name: nord\n  updated_at: t\nspec:\n  title: Nord\nstatus: {}\n";
        let kept = without_server_fields(printed.to_owned());
        assert!(
            !kept.contains("updated_at") && !kept.contains("status"),
            "{kept}"
        );
        assert!(kept.contains("name: nord"), "{kept}");
        let written = "# a comment the file keeps\nkind: Theme\n";
        assert_eq!(without_server_fields(written.to_owned()), written);
    }

    #[test]
    fn finds_the_content_a_manifest_names() {
        let page = "kind: Page\nmetadata:\n  name: about\nspec:\n  content:\n    file: about.md\n";
        assert_eq!(
            named_content("pages/about.yaml", page).as_deref(),
            Some("pages/about.md")
        );
        let inline = "kind: Page\nspec:\n  content:\n    inline: x\n";
        assert_eq!(named_content("pages/a.yaml", inline), None);
    }
}
