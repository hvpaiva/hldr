//! A content tree: every file of one revision, parsed, and the checks that
//! only hold across files.
//!
//! The indexer and `hldr validate` read trees through here, so a checkout
//! that validates is one the server indexes. Page types come first: the
//! PageKinds and Collections decide what the pages of each directory are.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::Error;
use crate::manifest::{
    self, Kind, Location, Manifest, Page, RESERVED_NAMES, Registry, check_collection_fields,
};
use crate::spec::{
    CollectionSpec, NavCommand, NavLink, NavSpec, PageKindSpec, PageStyle, SiteSpec, ThemeSpec,
};
use crate::template::{self, Reference};

/// A manifest, parsed, with the markdown it names.
#[derive(Debug, Clone)]
pub struct File {
    /// Path under the content root.
    pub path: PathBuf,
    /// The manifest as stored.
    pub bytes: Vec<u8>,
    pub manifest: Manifest,
    /// The content file a page names, as stored.
    pub content: Option<Content>,
}

/// A page's content file.
#[derive(Debug, Clone)]
pub struct Content {
    /// Path under the content root.
    pub path: PathBuf,
    pub text: String,
}

impl File {
    /// What the indexer hashes to skip unchanged files: the manifest and the
    /// content file it names.
    pub fn source(&self) -> Vec<u8> {
        let mut source = self.bytes.clone();
        if let Some(content) = &self.content {
            source.push(0);
            source.extend_from_slice(content.text.as_bytes());
        }
        source
    }

    /// A page's markdown: its content file, or its inline content.
    pub fn markdown(&self) -> Option<&str> {
        if let Some(content) = &self.content {
            return Some(&content.text);
        }
        match &self.manifest {
            Manifest::Page(page) => page.spec.content.as_ref()?.inline.as_deref(),
            _ => None,
        }
    }
}

/// The manifests of a tree, in path order.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub files: Vec<File>,
}

impl Tree {
    /// Reads and parses every manifest under `root` with the content it
    /// names, then checks the tree as a whole. Every problem is reported, not
    /// only the first; the checks across files run once every file parses,
    /// so a file that failed is never also reported as missing.
    pub fn read(root: &Path) -> Result<Self, Error> {
        if !root.is_dir() {
            return Err(Error::file(root, "content directory does not exist"));
        }
        let mut problems = Vec::new();
        let mut manifests = Vec::new();
        let mut contents = BTreeSet::new();
        for path in paths(root)? {
            match manifest::locate(&path) {
                Ok(Some(Location::Content { .. })) => {
                    contents.insert(path);
                }
                Ok(Some(location)) => manifests.push((path, location)),
                Ok(None) => {}
                Err(err) => problems.push(err),
            }
        }

        let (types, pages): (Vec<_>, Vec<_>) = manifests.into_iter().partition(|(_, location)| {
            !matches!(
                location,
                Location::Member { .. }
                    | Location::Manifest {
                        kind: Kind::Page,
                        ..
                    }
            )
        });
        let declared: HashSet<PathBuf> = types.iter().map(|(path, _)| path.clone()).collect();
        let mut skipped = HashSet::new();
        let mut files = Vec::new();
        for (path, _) in &types {
            match read_manifest(root, path, &Registry::default()) {
                Ok(file) => files.push(file),
                Err(err) => problems.push(err),
            }
        }
        let registry = registry(&files);
        for (path, location) in &pages {
            if let Location::Member { collection, .. } = location {
                // A member of a collection or type that failed to parse is not
                // reported again: its collection's problem says it all.
                let home = PathBuf::from(manifest::path(Kind::Collection, Some(collection)));
                let parsed = registry.collection(collection);
                if declared.contains(&home)
                    && parsed.is_none_or(|home| registry.kind(&home.kind).is_none())
                    && !problems.is_empty()
                {
                    skipped.insert(collection.clone());
                    continue;
                }
            }
            match read_manifest(root, path, &registry) {
                Ok(file) => files.push(file),
                Err(err) => problems.push(err),
            }
        }

        let mut referenced = HashSet::new();
        for file in &mut files {
            let Manifest::Page(page) = &file.manifest else {
                continue;
            };
            let Some(content_path) = page.content_path().map(PathBuf::from) else {
                continue;
            };
            referenced.insert(content_path.clone());
            match read_content(root, &content_path, &file.path, page, &registry) {
                Ok(content) => file.content = Some(content),
                Err(found) => problems.extend(found),
            }
        }
        let unread = |path: &PathBuf| {
            path.parent()
                .and_then(Path::to_str)
                .is_some_and(|dir| skipped.contains(dir))
        };
        for path in contents
            .iter()
            .filter(|path| !referenced.contains(*path) && !unread(path))
        {
            problems.push(Error::file(
                path,
                "no page names this as its content: set it as a manifest's spec.content.file, or remove it",
            ));
        }

        files.sort_by(|a, b| a.path.cmp(&b.path));
        let tree = Self { files };
        if problems.is_empty() {
            problems = tree.check();
        }
        Error::collect(problems).map(|()| tree)
    }

    /// The page types the tree declares.
    pub fn registry(&self) -> Registry {
        registry(&self.files)
    }

    /// What no single file can get wrong: the singletons and the pages the
    /// site needs exist, page types and collections pair up, names do not
    /// clash, and every reference resolves.
    pub fn check(&self) -> Vec<Error> {
        let mut problems = Vec::new();
        for kind in [Kind::Site, Kind::Profile, Kind::Nav] {
            if !self
                .files
                .iter()
                .any(|file| file.manifest.kind() == kind.as_str())
            {
                problems.push(Error::file(
                    manifest::path(kind, None),
                    format!("missing: every content tree has one {}", kind.as_str()),
                ));
            }
        }
        for (name, needed) in [
            ("index", "the site serves it at /"),
            ("not-found", "every 404 shows it"),
        ] {
            if self.page(name).is_none() {
                problems.push(Error::file(
                    manifest::path(Kind::Page, Some(name)),
                    format!("missing: {needed}"),
                ));
            }
        }
        self.check_colorscheme(&mut problems);
        self.check_types(&mut problems);
        self.check_names(&mut problems);
        self.check_references(&mut problems);
        if let Some(site) = self.site() {
            match self.theme(&site.theme) {
                None => problems.push(Error::file(
                    manifest::path(Kind::Site, None),
                    format!(
                        "theme {} has no {}",
                        site.theme,
                        manifest::path(Kind::Theme, Some(&site.theme))
                    ),
                )),
                Some(theme) if theme.hidden => problems.push(Error::file(
                    manifest::path(Kind::Site, None),
                    format!(
                        "theme {} is hidden; the default theme must be listed",
                        site.theme
                    ),
                )),
                Some(_) => {}
            }
        }
        problems
    }

    pub fn site(&self) -> Option<&SiteSpec> {
        self.files.iter().find_map(|file| match &file.manifest {
            Manifest::Site(spec) => Some(spec),
            _ => None,
        })
    }

    pub fn nav(&self) -> Option<&NavSpec> {
        self.files.iter().find_map(|file| match &file.manifest {
            Manifest::Nav(spec) => Some(spec),
            _ => None,
        })
    }

    /// A page under `pages/`.
    pub fn page(&self, name: &str) -> Option<&Page> {
        self.pages()
            .find(|page| page.collection.is_none() && page.name == name)
    }

    /// Every page, of any type.
    pub fn pages(&self) -> impl Iterator<Item = &Page> {
        self.files.iter().filter_map(|file| match &file.manifest {
            Manifest::Page(page) => Some(page),
            _ => None,
        })
    }

    pub fn kinds(&self) -> impl Iterator<Item = (&str, &PageKindSpec)> {
        self.files.iter().filter_map(|file| match &file.manifest {
            Manifest::PageKind { name, spec } => Some((name.as_str(), spec)),
            _ => None,
        })
    }

    pub fn collections(&self) -> impl Iterator<Item = (&str, &CollectionSpec)> {
        self.files.iter().filter_map(|file| match &file.manifest {
            Manifest::Collection { name, spec } => Some((name.as_str(), spec)),
            _ => None,
        })
    }

    fn theme(&self, name: &str) -> Option<&ThemeSpec> {
        self.files.iter().find_map(|file| match &file.manifest {
            Manifest::Theme { name: n, spec } if n == name => Some(spec),
            _ => None,
        })
    }

    fn collection(&self, name: &str) -> Option<&CollectionSpec> {
        self.collections()
            .find_map(|(n, spec)| (n == name).then_some(spec))
    }

    fn check_colorscheme(&self, problems: &mut Vec<Error>) {
        let pickers: Vec<&Page> = self
            .pages()
            .filter(|page| page.spec.style == PageStyle::Colorscheme)
            .collect();
        if pickers.len() > 1 {
            let paths: Vec<String> = pickers.iter().map(|page| page.path()).collect();
            problems.push(Error::file(
                &paths[0],
                format!(
                    "{} pages have style colorscheme ({}); there is one picker",
                    pickers.len(),
                    paths.join(", ")
                ),
            ));
        }
        let listed = self.nav().is_some_and(|nav| {
            nav.sections
                .iter()
                .flat_map(|section| &section.items)
                .any(|item| item.command == Some(NavCommand::Colorscheme))
        });
        if listed && pickers.is_empty() {
            problems.push(Error::file(
                manifest::path(Kind::Nav, None),
                "the colorscheme command needs a page whose spec.style is colorscheme",
            ));
        }
    }

    fn check_types(&self, problems: &mut Vec<Error>) {
        for (name, spec) in self.collections() {
            let at = manifest::path(Kind::Collection, Some(name));
            match self.kinds().find(|(_, kind)| kind.names.kind == spec.kind) {
                None => problems.push(Error::file(
                    &at,
                    format!("kind {} is declared by no page type in kinds/", spec.kind),
                )),
                Some((_, kind)) => {
                    if let Err(message) = check_collection_fields(spec, kind) {
                        problems.push(Error::file(&at, message));
                    }
                }
            }
        }
        for (name, kind) in self.kinds() {
            let homes: Vec<&str> = self
                .collections()
                .filter(|(_, spec)| spec.kind == kind.names.kind)
                .map(|(name, _)| name)
                .collect();
            let at = manifest::path(Kind::PageKind, Some(name));
            match homes.as_slice() {
                [_] => {}
                [] => problems.push(Error::file(
                    &at,
                    format!(
                        "no collection holds {}; add one in collections/",
                        kind.names.kind
                    ),
                )),
                many => problems.push(Error::file(
                    &at,
                    format!(
                        "{} is held by collections {}; a page type has one home",
                        kind.names.kind,
                        many.join(" and ")
                    ),
                )),
            }
        }
    }

    fn check_names(&self, problems: &mut Vec<Error>) {
        let mut nouns: BTreeMap<&str, &str> = BTreeMap::new();
        let mut kinds: BTreeMap<&str, &str> = BTreeMap::new();
        for (name, kind) in self.kinds() {
            let at = manifest::path(Kind::PageKind, Some(name));
            if let Some(other) = kinds.insert(&kind.names.kind, name) {
                problems.push(Error::file(
                    &at,
                    format!(
                        "kind {} is also declared by kinds/{other}.yaml",
                        kind.names.kind
                    ),
                ));
            }
            let own: BTreeSet<&str> = [&kind.names.singular, &kind.names.plural]
                .into_iter()
                .chain(&kind.names.short_names)
                .map(String::as_str)
                .collect();
            for noun in own {
                if let Some(other) = nouns.insert(noun, name) {
                    problems.push(Error::file(
                        &at,
                        format!("{noun} also names the page type in kinds/{other}.yaml"),
                    ));
                }
            }
        }
        for page in self.pages().filter(|page| page.collection.is_none()) {
            if RESERVED_NAMES.contains(&page.name.as_str()) {
                problems.push(Error::file(
                    page.path(),
                    format!(
                        "{} is reserved: no page can be named {}",
                        page.name,
                        RESERVED_NAMES.join(", ")
                    ),
                ));
            }
            if self.collection(&page.name).is_some() {
                problems.push(Error::file(
                    page.path(),
                    format!(
                        "/{} is also collections/{}.yaml; a page and a collection cannot share a name",
                        page.name, page.name
                    ),
                ));
            }
        }
    }

    fn check_references(&self, problems: &mut Vec<Error>) {
        let resolve = |at: &str, what: &str, reference: &Reference, problems: &mut Vec<Error>| {
            if let Err(message) = self.resolve(reference) {
                problems.push(Error::file(at, format!("{what}{message}")));
            }
        };
        if let Some(nav) = self.nav() {
            let at = manifest::path(Kind::Nav, None);
            for section in &nav.sections {
                let texts = section
                    .title
                    .iter()
                    .chain(section.items.iter().flat_map(|item| {
                        let literal = match &item.link {
                            Some(NavLink::Literal(link)) => vec![&link.label, &link.url],
                            _ => Vec::new(),
                        };
                        item.label.iter().chain(literal)
                    }));
                for text in texts {
                    for reference in template::check_text(text).unwrap_or_default() {
                        resolve(&at, "", &reference, problems);
                    }
                }
                for item in &section.items {
                    if let Some(name) = &item.page {
                        match self.page(name) {
                            None => problems
                                .push(Error::file(&at, format!("page {name} is not in pages/"))),
                            Some(page) if page.route().is_none() => problems.push(Error::file(
                                &at,
                                format!("page {name} has no route to link"),
                            )),
                            Some(_) => {}
                        }
                    }
                    if let Some(name) = &item.collection
                        && self.collection(name).is_none()
                    {
                        problems.push(Error::file(
                            &at,
                            format!("collection {name} is not in collections/"),
                        ));
                    }
                    if let Some(NavLink::Path(path)) = &item.link {
                        resolve(&at, "", &Reference::Value(path.clone()), problems);
                    }
                }
            }
            if let Some(count) = &nav.statusline.count
                && self.collection(count).is_none()
            {
                problems.push(Error::file(
                    &at,
                    format!("statusline.count: collection {count} is not in collections/"),
                ));
            }
        }
        for (name, kind) in self.kinds() {
            let at = manifest::path(Kind::PageKind, Some(name));
            let references = kind
                .footer
                .as_deref()
                .map(template::check_markdown)
                .and_then(Result::ok)
                .unwrap_or_default();
            for (line, reference) in &references {
                resolve(&at, &format!("footer line {line}: "), reference, problems);
            }
        }
        for file in &self.files {
            let Manifest::Page(page) = &file.manifest else {
                continue;
            };
            let at = file.path.to_string_lossy();
            for (field, text) in [
                ("title", Some(page.title())),
                ("description", page.description()),
            ] {
                let references = text
                    .map(template::check_text)
                    .and_then(Result::ok)
                    .unwrap_or_default();
                for reference in &references {
                    resolve(&at, &format!("metadata.{field}: "), reference, problems);
                }
            }
            let (source, where_) = match &file.content {
                Some(content) => (Some(content.text.as_str()), content.path.to_string_lossy()),
                None => (file.markdown(), at.clone()),
            };
            let references = source
                .map(template::check_markdown)
                .and_then(Result::ok)
                .unwrap_or_default();
            let inline = if file.content.is_some() {
                ""
            } else {
                "spec.content.inline "
            };
            for (line, reference) in &references {
                resolve(
                    &where_,
                    &format!("{inline}line {line}: "),
                    reference,
                    problems,
                );
            }
        }
    }

    /// Checks what a reference names exists and fits. Profile and page
    /// references are checked with their file; site values and collections
    /// only exist across files.
    fn resolve(&self, reference: &Reference) -> Result<(), String> {
        match reference {
            Reference::Value(path) => match self.site_value(path)? {
                None | Some(Value::String(_) | Value::Number(_) | Value::Bool(_)) => Ok(()),
                Some(Value::Array(_)) => {
                    Err(format!("`{path}` is a list, which does not fit in a line"))
                }
                Some(_) => Err(format!(
                    "`{path}` holds a map, which does not fit in a line"
                )),
            },
            Reference::Clone(path) => match self.site_value(path)? {
                None | Some(Value::String(_)) => Ok(()),
                Some(_) => Err(format!("`{path}` must hold a URL to clone")),
            },
            Reference::Banner(path) => {
                if !path.starts_with("site.values.") {
                    return Err(format!("`{path}`: banner art lives in site.values"));
                }
                let art = |key| {
                    self.site_value(path)
                        .ok()
                        .flatten()
                        .and_then(|value| value.get(key))
                        .is_some_and(Value::is_string)
                };
                if art("art") && art("alt") {
                    Ok(())
                } else {
                    Err(format!("`{path}` must hold art and alt, both text"))
                }
            }
            Reference::Collection { name, filter } => {
                let Some(collection) = self.collection(name) else {
                    return Err(format!("collection {name} is not in collections/"));
                };
                match (
                    filter,
                    self.kinds().find(|(_, k)| k.names.kind == collection.kind),
                ) {
                    (Some(field), Some((_, kind))) if !kind.fields.contains_key(field) => Err(
                        format!("`where={field}`: {} has no field {field}", collection.kind),
                    ),
                    _ => Ok(()),
                }
            }
        }
    }

    /// The site value a path names: `None` for paths into the profile or the
    /// page, which their files check; an error for a site value not set.
    fn site_value(&self, path: &str) -> Result<Option<&Value>, String> {
        let Some(key) = path.strip_prefix("site.values.") else {
            return Ok(None);
        };
        let values = self.site().map(|site| &site.values);
        let found = values.and_then(|values| {
            let mut keys = key.split('.');
            let first = values.get(keys.next()?)?;
            keys.try_fold(first, |node, key| node.get(key))
        });
        match found {
            Some(Value::Null) | None => Err(format!("`{path}` is not set in site.yaml")),
            Some(value) => Ok(Some(value)),
        }
    }
}

/// The page types a checkout declares in `kinds/` and `collections/`, to
/// check one of its files alone.
pub fn read_registry(root: &Path) -> Result<Registry, Error> {
    let mut files = Vec::new();
    let mut problems = Vec::new();
    for path in paths(root)? {
        let is_type = matches!(
            manifest::locate(&path),
            Ok(Some(Location::Manifest {
                kind: Kind::PageKind | Kind::Collection,
                ..
            }))
        );
        if is_type {
            match read_manifest(root, &path, &Registry::default()) {
                Ok(file) => files.push(file),
                Err(err) => problems.push(err),
            }
        }
    }
    Error::collect(problems).map(|()| registry(&files))
}

fn registry(files: &[File]) -> Registry {
    let kinds = files.iter().filter_map(|file| match &file.manifest {
        Manifest::PageKind { spec, .. } => Some(spec.clone()),
        _ => None,
    });
    let collections = files.iter().filter_map(|file| match &file.manifest {
        Manifest::Collection { name, spec } => Some((name.clone(), spec.clone())),
        _ => None,
    });
    Registry::new(kinds, collections)
}

fn read_manifest(root: &Path, path: &Path, registry: &Registry) -> Result<File, Error> {
    let bytes = fs::read(root.join(path)).map_err(|err| Error::file(path, err.to_string()))?;
    let manifest = manifest::parse(path, &bytes, registry)?;
    Ok(File {
        path: path.to_owned(),
        bytes,
        manifest,
        content: None,
    })
}

/// Reads the content file a page names and checks it: markdown without a
/// frontmatter, whose directives parse and fit the page's type.
fn read_content(
    root: &Path,
    path: &Path,
    manifest: &Path,
    page: &Page,
    registry: &Registry,
) -> Result<Content, Vec<Error>> {
    let fail = |message: String| vec![Error::file(path, message)];
    let bytes = match fs::read(root.join(path)) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(vec![Error::file(
                manifest,
                format!(
                    "spec.content.file names {}, which does not exist",
                    path.display()
                ),
            )]);
        }
        Err(err) => return Err(fail(err.to_string())),
    };
    let text = String::from_utf8(bytes).map_err(|_| fail("not valid UTF-8".to_owned()))?;
    if text.starts_with("---\n") || text.starts_with("---\r\n") {
        return Err(fail(format!(
            "content has no frontmatter: its fields belong in {}",
            manifest.display()
        )));
    }
    registry
        .check_content(&page.kind, &text)
        .map_err(|problems| {
            problems
                .into_iter()
                .map(|message| Error::file(path, message))
                .collect::<Vec<_>>()
        })?;
    Ok(Content {
        path: path.to_owned(),
        text,
    })
}

/// Files under `root`, relative to it and in order, skipping hidden entries
/// such as `.git` and `.github`.
pub fn paths(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let full = root.join(&dir);
        let entries = fs::read_dir(&full).map_err(|err| Error::file(&full, err.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|err| Error::file(&full, err.to_string()))?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = dir.join(entry.file_name());
            let kind = entry
                .file_type()
                .map_err(|err| Error::file(&path, err.to_string()))?;
            if kind.is_dir() {
                pending.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{self, TREE};

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        testing::write(dir.path(), files);
        dir
    }

    fn valid() -> Vec<(&'static str, String)> {
        TREE.iter()
            .map(|(path, text)| (*path, (*text).to_owned()))
            .collect()
    }

    /// Each problem reading `files` finds, as text.
    fn problems(files: &[(&str, String)]) -> Vec<String> {
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        let dir = tree(&files);
        let err = Tree::read(dir.path()).unwrap_err();
        err.problems().iter().map(ToString::to_string).collect()
    }

    fn with(
        mut files: Vec<(&'static str, String)>,
        path: &'static str,
        text: &str,
    ) -> Vec<(&'static str, String)> {
        files.retain(|(p, _)| *p != path);
        files.push((path, text.to_owned()));
        files
    }

    fn without(mut files: Vec<(&'static str, String)>, path: &str) -> Vec<(&'static str, String)> {
        files.retain(|(p, _)| *p != path);
        files
    }

    #[test]
    fn reads_the_manifests_and_their_content() {
        let mut files: Vec<(&str, &str)> = TREE.to_vec();
        files.extend([
            ("README.md", "# content\n"),
            (".github/workflows/validate.yml", "on: push\n"),
            ("projects/notes.txt", "not content\n"),
        ]);
        let dir = tree(&files);
        let read = Tree::read(dir.path()).unwrap();
        let paths: Vec<&Path> = read.files.iter().map(|f| f.path.as_path()).collect();
        assert_eq!(
            paths,
            [
                "collections/projects.yaml",
                "kinds/project.yaml",
                "nav.yaml",
                "pages/about.yaml",
                "pages/index.yaml",
                "pages/not-found.yaml",
                "pages/theme.yaml",
                "profile.yaml",
                "projects/atlas.yaml",
                "site.yaml",
                "themes/nord.yaml",
            ]
            .map(Path::new)
        );
        let atlas = read
            .files
            .iter()
            .find(|f| f.path == Path::new("projects/atlas.yaml"))
            .unwrap();
        assert_eq!(atlas.markdown(), Some(testing::ATLAS_MD));
        let not_found = read
            .files
            .iter()
            .find(|f| f.path == Path::new("pages/not-found.yaml"))
            .unwrap();
        assert!(not_found.markdown().unwrap().contains("start here"));
        assert!(atlas.source().ends_with(testing::ATLAS_MD.as_bytes()));
    }

    #[test]
    fn reports_every_file_that_fails() {
        let files = with(valid(), "projects/bad.yaml", "kind: Project\n");
        let files = with(files, "themes/odd.yaml", "kind: Site\n");
        let err = problems(&files);
        assert_eq!(err.len(), 2, "{err:?}");
        assert!(
            err.iter().any(|e| e.starts_with("projects/bad.yaml: ")),
            "{err:?}"
        );
        assert!(
            err.iter().any(|e| e.starts_with("themes/odd.yaml: ")),
            "{err:?}"
        );
    }

    #[test]
    fn requires_the_singletons_and_the_pages_the_site_needs() {
        let files = without(without(valid(), "nav.yaml"), "pages/not-found.yaml");
        let err = problems(&files);
        assert_eq!(
            err,
            [
                "nav.yaml: missing: every content tree has one Nav",
                "pages/not-found.yaml: missing: every 404 shows it"
            ]
        );
    }

    #[test]
    fn the_default_theme_must_exist_and_be_listed() {
        let err = problems(&without(valid(), "themes/nord.yaml"));
        assert_eq!(err, ["site.yaml: theme nord has no themes/nord.yaml"]);
        let hidden = testing::THEME.replace("  dark: true\n", "  dark: true\n  hidden: true\n");
        let err = problems(&with(valid(), "themes/nord.yaml", &hidden));
        assert_eq!(
            err,
            ["site.yaml: theme nord is hidden; the default theme must be listed"]
        );
    }

    #[test]
    fn a_file_that_fails_is_not_also_missing() {
        let files = with(valid(), "site.yaml", "kind: Site\nspec:\n  title: [\n");
        let err = problems(&files);
        assert_eq!(err.len(), 1, "{err:?}");
        assert!(err[0].starts_with("site.yaml: "), "{err:?}");

        let files = with(valid(), "collections/projects.yaml", "kind: Collection\n");
        let err = problems(&files);
        assert_eq!(err.len(), 1, "a broken collection hides its pages: {err:?}");
    }

    #[test]
    fn content_files_are_checked_and_accounted_for() {
        let err = problems(&without(valid(), "pages/about.md"));
        assert_eq!(
            err,
            ["pages/about.yaml: spec.content.file names pages/about.md, which does not exist"]
        );

        let err = problems(&with(valid(), "pages/draft.md", "# stray\n"));
        assert_eq!(err.len(), 1, "{err:?}");
        assert!(
            err[0].starts_with("pages/draft.md: no page names this"),
            "{err:?}"
        );

        let err = problems(&with(
            valid(),
            "pages/about.md",
            "---\ntitle: x\n---\n# about\n",
        ));
        assert!(err[0].contains("content has no frontmatter"), "{err:?}");

        let err = problems(&with(valid(), "projects/atlas.md", "{{ page.nope }}\n"));
        assert_eq!(
            err,
            ["projects/atlas.md: line 1: `page.nope`: the page has no field nope"]
        );
    }

    #[test]
    fn page_types_and_collections_pair_up() {
        let err = problems(&without(valid(), "collections/projects.yaml"));
        assert!(
            err.iter()
                .any(|e| e.contains("declare it in collections/projects.yaml")),
            "{err:?}"
        );

        let files = without(without(valid(), "projects/atlas.yaml"), "projects/atlas.md");
        let files = without(files, "collections/projects.yaml");
        let files = with(
            files,
            "nav.yaml",
            &testing::NAV
                .replace(
                    "        - collection: projects\n          open_max: 8\n",
                    "",
                )
                .replace("  statusline:\n    count: projects\n", ""),
        );
        let files = with(files, "pages/index.md", "# index\n");
        let err = problems(&files);
        assert_eq!(
            err,
            ["kinds/project.yaml: no collection holds Project; add one in collections/"]
        );

        let twin = testing::PROJECTS.replace("name: projects", "name: work");
        let err = problems(&with(valid(), "collections/work.yaml", &twin));
        assert_eq!(
            err,
            [
                "kinds/project.yaml: Project is held by collections projects and work; a page type has one home"
            ]
        );

        let stray = testing::PROJECTS
            .replace("name: projects", "name: posts")
            .replace("kind: Project", "kind: Post");
        let err = problems(&with(valid(), "collections/posts.yaml", &stray));
        assert_eq!(
            err,
            ["collections/posts.yaml: kind Post is declared by no page type in kinds/"]
        );
    }

    #[test]
    fn names_do_not_clash() {
        let page = "kind: Page\nmetadata:\n  name: projects\n  title: p\nspec:\n  content:\n    inline: x\n";
        let err = problems(&with(valid(), "pages/projects.yaml", page));
        assert_eq!(err.len(), 1, "{err:?}");
        assert!(
            err[0].contains("a page and a collection cannot share a name"),
            "{err:?}"
        );

        let reserved = page.replace("name: projects", "name: help");
        let err = problems(&with(valid(), "pages/help.yaml", &reserved));
        assert!(
            err[0].starts_with("pages/help.yaml: help is reserved"),
            "{err:?}"
        );
    }

    #[test]
    fn references_resolve() {
        for (path, from, to, expected) in [
            (
                "pages/about.md",
                "{{ site.values.about }}",
                "{{ site.values.missing }}",
                "pages/about.md: line 3: `site.values.missing` is not set in site.yaml",
            ),
            (
                "pages/index.md",
                "{{ banner site.values.banner }}",
                "{{ banner site.values.about }}",
                "pages/index.md: line 1: `site.values.about` must hold art and alt, both text",
            ),
            (
                "pages/index.md",
                "{{ collection projects where=highlight limit=3 }}",
                "{{ collection blog }}",
                "pages/index.md: line 6: collection blog is not in collections/",
            ),
            (
                "pages/index.md",
                "where=highlight",
                "where=rank",
                "pages/index.md: line 6: `where=rank`: Project has no field rank",
            ),
            (
                "pages/not-found.yaml",
                "{{ link / README.md }}",
                "{{ site.values.nope }}",
                "pages/not-found.yaml: spec.content.inline line 1: `site.values.nope` is not set",
            ),
            (
                "pages/about.yaml",
                "{{ profile.headline }}",
                "{{ site.values.banner }}",
                "pages/about.yaml: metadata.description: `site.values.banner` holds a map",
            ),
            (
                "nav.yaml",
                "- page: about",
                "- page: contact",
                "nav.yaml: page contact is not in pages/",
            ),
            (
                "nav.yaml",
                "count: projects",
                "count: posts",
                "nav.yaml: statusline.count: collection posts is not in collections/",
            ),
            (
                "nav.yaml",
                "{{ site.title }}",
                "{{ site.values.nope }}",
                "nav.yaml: `site.values.nope` is not set in site.yaml",
            ),
        ] {
            let text = TREE.iter().find(|(p, _)| *p == path).unwrap().1;
            let changed = text.replace(from, to);
            assert_ne!(changed, text, "{from}");
            let err = problems(&with(valid(), path, &changed));
            assert_eq!(err.len(), 1, "{to}: {err:?}");
            assert!(err[0].starts_with(expected), "{to}: {err:?}");
        }
    }

    #[test]
    fn one_colorscheme_page_at_most_and_one_if_the_nav_lists_it() {
        let err = problems(&without(valid(), "pages/theme.yaml"));
        assert_eq!(
            err,
            ["nav.yaml: the colorscheme command needs a page whose spec.style is colorscheme"]
        );
        let twin = testing::THEME_PAGE.replace("name: theme", "name: colors");
        let err = problems(&with(valid(), "pages/colors.yaml", &twin));
        assert!(err[0].contains("2 pages have style colorscheme"), "{err:?}");
    }

    #[test]
    fn a_missing_directory_is_an_error() {
        let err = Tree::read(Path::new("/nonexistent/content")).unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
    }
}
