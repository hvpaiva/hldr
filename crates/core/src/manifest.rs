//! Content files as the content repository stores them.
//!
//! Every file is a YAML manifest with `kind`, `metadata` and `spec`, except
//! the markdown a page's manifest names as its content. Where a file sits
//! must agree with the kind it declares. The indexer and the CLI share this
//! parser, so what `hldr edit` accepts is what the server indexes.
//!
//! Pages invert Kubernetes on purpose: their `metadata` describes them, and
//! is what their frontmatter shows, while `spec` says how they are built and
//! is the same for every page type.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::Error;
use crate::spec::{
    CollectionSpec, FieldSpec, FieldType, NavLink, NavSpec, PageContent, PageKindSpec, PageSpec,
    PageStyle, ProfileSpec, SiteSpec, ThemeSpec,
};
use crate::template;

/// A kind the code defines. Page types that content defines, such as
/// `Project`, are declared by a [`PageKindSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Site,
    Profile,
    Nav,
    PageKind,
    Collection,
    Page,
    Theme,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Self::Site,
        Self::Profile,
        Self::Nav,
        Self::PageKind,
        Self::Collection,
        Self::Page,
        Self::Theme,
    ];

    /// The kind a file declares, such as `Theme`.
    pub fn from_declared(kind: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|known| known.as_str() == kind)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Site => "Site",
            Self::Profile => "Profile",
            Self::Nav => "Nav",
            Self::PageKind => "PageKind",
            Self::Collection => "Collection",
            Self::Page => "Page",
            Self::Theme => "Theme",
        }
    }

    /// Path under the content root; `{name}` stands for the resource name.
    pub fn path_template(self) -> &'static str {
        match self {
            Self::Site => "site.yaml",
            Self::Profile => "profile.yaml",
            Self::Nav => "nav.yaml",
            Self::PageKind => "kinds/{name}.yaml",
            Self::Collection => "collections/{name}.yaml",
            Self::Page => "pages/{name}.yaml",
            Self::Theme => "themes/{name}.yaml",
        }
    }

    /// Exactly one resource of this kind exists, and it has no name.
    pub fn is_singleton(self) -> bool {
        matches!(self, Self::Site | Self::Profile | Self::Nav)
    }

    /// The directory a named kind lives in.
    fn dir(self) -> Option<&'static str> {
        match self {
            Self::PageKind => Some("kinds"),
            Self::Collection => Some("collections"),
            Self::Page => Some("pages"),
            Self::Theme => Some("themes"),
            Self::Site | Self::Profile | Self::Nav => None,
        }
    }
}

/// Names no page or collection can take: the code serves routes there.
pub const RESERVED_NAMES: [&str; 5] = ["api", "help", "health", "healthz", "readyz"];

/// Further names no collection can take: `/theme/<name>` is code, and the
/// others are directories of the layout.
pub const RESERVED_COLLECTIONS: [&str; 5] = ["theme", "pages", "kinds", "collections", "themes"];

/// Resource names the API has before any page type: no page type can take
/// one as its plural, singular or short name.
pub const BUILTIN_RESOURCES: [&str; 17] = [
    "site",
    "profile",
    "nav",
    "pagekinds",
    "pagekind",
    "collections",
    "collection",
    "pages",
    "page",
    "themes",
    "theme",
    "th",
    "api-resources",
    "schema",
    "sync",
    "list",
    "status",
];

/// What a content path holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// A manifest of a built-in kind; `name` is `None` for singletons.
    Manifest { kind: Kind, name: Option<String> },
    /// A manifest in a collection's directory: a page of its type.
    Member { collection: String, name: String },
    /// A markdown file beside page manifests, which one of them must name
    /// as its content.
    Content { dir: String, file: String },
}

/// Resolves a path relative to the content root. Files outside the layout,
/// such as the README at the root, hold nothing and resolve to `None`.
pub fn locate(path: &Path) -> Result<Option<Location>, Error> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| Error::file(path, "path is not valid UTF-8"))?,
            ),
            _ => {
                return Err(Error::file(
                    path,
                    "path must be relative to the content root",
                ));
            }
        }
    }
    let singleton = |kind| Ok(Some(Location::Manifest { kind, name: None }));
    match parts.as_slice() {
        ["site.yaml"] => singleton(Kind::Site),
        ["profile.yaml"] => singleton(Kind::Profile),
        ["nav.yaml"] => singleton(Kind::Nav),
        ["profile.md"] => Err(Error::file(
            path,
            "the profile is profile.yaml now, and the about text pages/about.md",
        )),
        [dir, file] => {
            let builtin = Kind::ALL.into_iter().find(|kind| kind.dir() == Some(*dir));
            if let Some(name) = file.strip_suffix(".yaml") {
                let name = validate_name(path, name)?.to_owned();
                return Ok(Some(match builtin {
                    Some(kind) => Location::Manifest {
                        kind,
                        name: Some(name),
                    },
                    None if is_valid_name(dir) => Location::Member {
                        collection: (*dir).to_owned(),
                        name,
                    },
                    None => return Ok(None),
                }));
            }
            let content_dir = matches!(builtin, Some(Kind::Page) | None) && is_valid_name(dir);
            if content_dir && file.ends_with(".md") {
                return Ok(Some(Location::Content {
                    dir: (*dir).to_owned(),
                    file: (*file).to_owned(),
                }));
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// Path of a built-in resource under the content root.
pub fn path(kind: Kind, name: Option<&str>) -> String {
    match name {
        Some(name) => kind.path_template().replace("{name}", name),
        None => kind.path_template().to_owned(),
    }
}

/// Whether `name` can name a resource: `[a-z0-9][a-z0-9-]*`. Names end up
/// in file paths and URL paths, so nothing else is accepted.
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c.is_ascii_lowercase() || c.is_ascii_digit() || (i > 0 && c == '-'))
}

fn validate_name<'a>(path: &Path, name: &'a str) -> Result<&'a str, Error> {
    if is_valid_name(name) {
        Ok(name)
    } else {
        Err(Error::file(path, "name must match [a-z0-9][a-z0-9-]*"))
    }
}

/// The page types a tree declares: they decide which directories hold
/// pages and what those pages declare.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    /// By the `kind` their pages declare.
    kinds: BTreeMap<String, PageKindSpec>,
    collections: BTreeMap<String, CollectionSpec>,
}

impl Registry {
    pub fn new(
        kinds: impl IntoIterator<Item = PageKindSpec>,
        collections: impl IntoIterator<Item = (String, CollectionSpec)>,
    ) -> Self {
        Self {
            kinds: kinds
                .into_iter()
                .map(|spec| (spec.names.kind.clone(), spec))
                .collect(),
            collections: collections.into_iter().collect(),
        }
    }

    /// A page type by the `kind` its pages declare.
    pub fn kind(&self, kind: &str) -> Option<&PageKindSpec> {
        self.kinds.get(kind)
    }

    pub fn collection(&self, name: &str) -> Option<&CollectionSpec> {
        self.collections.get(name)
    }

    /// Checks a page's markdown against the fields of its type, `kind` as
    /// the page declares it; a `Page` has none. Problems come back as
    /// `line N: ...`.
    pub fn check_content(&self, kind: &str, source: &str) -> Result<(), Vec<String>> {
        let empty = indexmap::IndexMap::new();
        let fields = self.kind(kind).map_or(&empty, |spec| &spec.fields);
        check_content(source, fields)
    }
}

/// A parsed manifest.
#[derive(Debug, Clone, PartialEq)]
pub enum Manifest {
    Site(SiteSpec),
    Profile(ProfileSpec),
    Nav(NavSpec),
    PageKind { name: String, spec: PageKindSpec },
    Collection { name: String, spec: CollectionSpec },
    Theme { name: String, spec: ThemeSpec },
    Page(Page),
}

/// A page of any type, as its manifest declares it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    /// `Page`, or what the pages of a type declare, such as `Project`.
    pub kind: String,
    pub name: String,
    /// The collection it belongs to; `None` for a page under `pages/`.
    pub collection: Option<String>,
    /// `title`, `description` and the type's fields, without `name` and
    /// without values left empty.
    pub metadata: Map<String, Value>,
    pub spec: PageSpec,
}

impl Page {
    pub fn title(&self) -> &str {
        self.metadata
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    pub fn description(&self) -> Option<&str> {
        self.metadata.get("description").and_then(Value::as_str)
    }

    /// Where the site serves it; `None` for `not-found`, the body of every
    /// 404, which has no route of its own.
    pub fn route(&self) -> Option<String> {
        match (&self.collection, self.name.as_str()) {
            (Some(collection), name) => Some(format!("/{collection}/{name}")),
            (None, "index") => Some("/".to_owned()),
            (None, "not-found") => None,
            (None, name) => Some(format!("/{name}")),
        }
    }

    /// Label in the tab, the tree and the statusline.
    pub fn buffer(&self) -> String {
        if let Some(buffer) = &self.spec.buffer {
            return buffer.clone();
        }
        match &self.collection {
            Some(collection) => format!("{collection}/{}.md", self.name),
            None => format!("{}.md", self.name),
        }
    }

    /// Path of the manifest under the content root.
    pub fn path(&self) -> String {
        format!(
            "{}/{}.yaml",
            self.collection.as_deref().unwrap_or("pages"),
            self.name
        )
    }

    /// Path of its content file under the content root, when it has one.
    pub fn content_path(&self) -> Option<String> {
        let file = self.spec.content.as_ref()?.file.as_ref()?;
        Some(format!(
            "{}/{file}",
            self.collection.as_deref().unwrap_or("pages")
        ))
    }

    /// The manifest as its file declares it.
    pub fn to_value(&self) -> Value {
        let mut metadata = Map::new();
        metadata.insert("name".to_owned(), Value::String(self.name.clone()));
        metadata.extend(self.metadata.clone());
        json!({
            "kind": self.kind,
            "metadata": metadata,
            "spec": self.spec,
        })
    }
}

impl Manifest {
    /// The kind the manifest declares, such as `Theme` or `Project`.
    pub fn kind(&self) -> &str {
        match self {
            Self::Site(_) => Kind::Site.as_str(),
            Self::Profile(_) => Kind::Profile.as_str(),
            Self::Nav(_) => Kind::Nav.as_str(),
            Self::PageKind { .. } => Kind::PageKind.as_str(),
            Self::Collection { .. } => Kind::Collection.as_str(),
            Self::Theme { .. } => Kind::Theme.as_str(),
            Self::Page(page) => &page.kind,
        }
    }

    /// The resource name; `None` for singletons.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Site(_) | Self::Profile(_) | Self::Nav(_) => None,
            Self::PageKind { name, .. }
            | Self::Collection { name, .. }
            | Self::Theme { name, .. } => Some(name),
            Self::Page(page) => Some(&page.name),
        }
    }

    /// Path of the manifest under the content root.
    pub fn path(&self) -> String {
        match self {
            Self::Site(_) => path(Kind::Site, None),
            Self::Profile(_) => path(Kind::Profile, None),
            Self::Nav(_) => path(Kind::Nav, None),
            Self::PageKind { name, .. } => path(Kind::PageKind, Some(name)),
            Self::Collection { name, .. } => path(Kind::Collection, Some(name)),
            Self::Theme { name, .. } => path(Kind::Theme, Some(name)),
            Self::Page(page) => page.path(),
        }
    }

    /// The manifest as its file declares it.
    pub fn to_value(&self) -> Result<Value, Error> {
        let named = |name: &str, spec: Value| json!({ "kind": self.kind(), "metadata": { "name": name }, "spec": spec });
        let singleton = |spec: Value| json!({ "kind": self.kind(), "spec": spec });
        Ok(match self {
            Self::Site(spec) => singleton(serde_json::to_value(spec)?),
            Self::Profile(spec) => singleton(serde_json::to_value(spec)?),
            Self::Nav(spec) => singleton(serde_json::to_value(spec)?),
            Self::PageKind { name, spec } => named(name, serde_json::to_value(spec)?),
            Self::Collection { name, spec } => named(name, serde_json::to_value(spec)?),
            Self::Theme { name, spec } => named(name, serde_json::to_value(spec)?),
            Self::Page(page) => page.to_value(),
        })
    }
}

/// Writes a manifest back as YAML.
pub fn render(manifest: &Manifest) -> Result<String, Error> {
    serde_saphyr::to_string(&manifest.to_value()?).map_err(|err| Error::Yaml(err.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SingletonFile<S> {
    #[allow(dead_code)]
    kind: String,
    spec: S,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedFile<S> {
    #[allow(dead_code)]
    kind: String,
    metadata: NameOnly,
    spec: S,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameOnly {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageFile {
    #[allow(dead_code)]
    kind: String,
    metadata: Map<String, Value>,
    spec: PageSpec,
}

/// The `kind` a manifest declares, read before knowing where it belongs.
pub fn declared_kind(source: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Head {
        kind: String,
    }
    serde_saphyr::from_str::<Head>(source)
        .ok()
        .map(|head| head.kind)
}

/// Parses a manifest. `path` is relative to the content root and decides
/// which kind the file must declare; `registry` holds the page types, which
/// decide what the pages of a collection declare.
pub fn parse(path: &Path, source: &[u8], registry: &Registry) -> Result<Manifest, Error> {
    let location = locate(path)?.ok_or_else(|| Error::file(path, "not a content file"))?;
    let text = std::str::from_utf8(source).map_err(|_| Error::file(path, "not valid UTF-8"))?;
    let fail = |message: String| Error::file(path, message);
    match location {
        Location::Content { .. } => Err(fail(
            "a markdown file is content: the manifest that names it is what gets parsed".into(),
        )),
        Location::Manifest { kind, name } => {
            expect_kind(path, text, kind.as_str())?;
            let manifest = match (kind, name) {
                (Kind::Site, _) => {
                    let spec: SiteSpec = singleton(path, text)?;
                    validate_name(path, &spec.theme)?;
                    Manifest::Site(spec)
                }
                (Kind::Profile, _) => Manifest::Profile(singleton(path, text)?),
                (Kind::Nav, _) => {
                    let spec: NavSpec = singleton(path, text)?;
                    check_nav(&spec).map_err(fail)?;
                    Manifest::Nav(spec)
                }
                (Kind::PageKind, Some(name)) => {
                    let spec: PageKindSpec = named(path, text, &name)?;
                    check_page_kind(&spec).map_err(fail)?;
                    Manifest::PageKind { name, spec }
                }
                (Kind::Collection, Some(name)) => {
                    let spec: CollectionSpec = named(path, text, &name)?;
                    check_collection(&name, &spec).map_err(fail)?;
                    Manifest::Collection { name, spec }
                }
                (Kind::Theme, Some(name)) => Manifest::Theme {
                    spec: named(path, text, &name)?,
                    name,
                },
                (Kind::Page, Some(name)) => {
                    Manifest::Page(page(path, text, name, None, "Page", None)?)
                }
                (kind, None) => return Err(fail(format!("{} without a name", kind.as_str()))),
            };
            Ok(manifest)
        }
        Location::Member { collection, name } => {
            let Some(home) = registry.collection(&collection) else {
                return Err(fail(format!(
                    "{collection}/ holds pages of no collection: declare it in collections/{collection}.yaml"
                )));
            };
            let Some(kind) = registry.kind(&home.kind) else {
                return Err(fail(format!(
                    "collection {collection} holds {}, which no page type in kinds/ declares",
                    home.kind
                )));
            };
            let declared = home.kind.clone();
            expect_kind(path, text, &declared)?;
            Ok(Manifest::Page(page(
                path,
                text,
                name,
                Some(collection),
                &declared,
                Some(kind),
            )?))
        }
    }
}

fn expect_kind(path: &Path, text: &str, expected: &str) -> Result<(), Error> {
    match declared_kind(text) {
        Some(declared) if declared != expected => Err(Error::file(
            path,
            format!("declares kind {declared}, but this path holds a {expected}"),
        )),
        _ => Ok(()),
    }
}

fn singleton<S: DeserializeOwned>(path: &Path, text: &str) -> Result<S, Error> {
    let file: SingletonFile<S> = from_yaml(path, text)?;
    Ok(file.spec)
}

fn named<S: DeserializeOwned>(path: &Path, text: &str, name: &str) -> Result<S, Error> {
    let file: NamedFile<S> = from_yaml(path, text)?;
    expect_name(path, &file.metadata.name, name)?;
    Ok(file.spec)
}

fn expect_name(path: &Path, declared: &str, name: &str) -> Result<(), Error> {
    if declared == name {
        Ok(())
    } else {
        Err(Error::file(
            path,
            format!("metadata.name is {declared}, but the file names it {name}"),
        ))
    }
}

fn from_yaml<T: DeserializeOwned>(path: &Path, text: &str) -> Result<T, Error> {
    serde_saphyr::from_str(text).map_err(|err| Error::file(path, err.to_string()))
}

fn page(
    path: &Path,
    text: &str,
    name: String,
    collection: Option<String>,
    kind: &str,
    spec: Option<&PageKindSpec>,
) -> Result<Page, Error> {
    let file: PageFile = from_yaml(path, text)?;
    let empty = indexmap::IndexMap::new();
    let fields = spec.map_or(&empty, |spec| &spec.fields);
    let mut problems = Vec::new();
    let mut metadata = Map::new();
    match file.metadata.get("name") {
        Some(Value::String(declared)) => {
            if let Err(err) = expect_name(path, declared, &name) {
                problems.push(err);
            }
        }
        Some(_) => problems.push(Error::file(path, "metadata.name must be a string")),
        None => problems.push(Error::file(path, "metadata.name is missing")),
    }
    for (key, value) in file.metadata {
        if key == "name" || value.is_null() {
            continue;
        }
        let checked = match key.as_str() {
            "title" | "description" => text_field(&key, &value),
            _ => match fields.get(&key) {
                Some(field) => field_value(&key, field, value.clone()),
                None if fields.is_empty() => Err(format!(
                    "metadata.{key}: a {kind} declares only name, title and description"
                )),
                None => Err(format!(
                    "metadata.{key} is not a field of {kind}; it has {}",
                    field_list(fields)
                )),
            },
        };
        match checked {
            Ok(value) => {
                metadata.insert(key, value);
            }
            Err(message) => problems.push(Error::file(path, message)),
        }
    }
    if !metadata.contains_key("title") {
        problems.push(Error::file(path, "metadata.title is missing"));
    }
    for (key, field) in fields {
        if field.required && !metadata.contains_key(key) {
            problems.push(Error::file(path, format!("metadata.{key} is missing")));
        }
    }
    if let Err(message) = check_page_spec(&file.spec, collection.is_some()) {
        problems.push(Error::file(path, message));
    }
    let page = Page {
        kind: kind.to_owned(),
        name,
        collection,
        metadata,
        spec: file.spec,
    };
    for (field, text) in [
        ("title", Some(page.title())),
        ("description", page.description()),
    ] {
        let Some(text) = text else { continue };
        let checked = template::check_text(text).and_then(|references| {
            references
                .iter()
                .try_for_each(|reference| check_page_reference(reference, fields))
        });
        if let Err(message) = checked {
            problems.push(Error::file(path, format!("metadata.{field}: {message}")));
        }
    }
    if let Some(inline) = page.spec.content.as_ref().and_then(|c| c.inline.as_deref())
        && let Err(found) = check_content(inline, fields)
    {
        problems.extend(
            found
                .into_iter()
                .map(|message| Error::file(path, format!("spec.content.inline {message}"))),
        );
    }
    Error::collect(problems).map(|()| page)
}

/// Checks a page's markdown, inline or from its file, against the fields
/// of its type. Problems come back as `line N: ...`.
pub fn check_content(
    source: &str,
    fields: &indexmap::IndexMap<String, FieldSpec>,
) -> Result<(), Vec<String>> {
    let references = template::check_markdown(source)?;
    let problems: Vec<String> = references
        .iter()
        .filter_map(|(line, reference)| {
            let err = check_page_reference(reference, fields).err()?;
            Some(format!("line {line}: {err}"))
        })
        .collect();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

/// A reference to `page.*` must name something the page type has, and a
/// value inserted into a line must print on one line.
fn check_page_reference(
    reference: &template::Reference,
    fields: &indexmap::IndexMap<String, FieldSpec>,
) -> Result<(), String> {
    let (path, scalar) = match reference {
        template::Reference::Value(path) | template::Reference::Clone(path) => (path, true),
        template::Reference::Banner(path) => (path, false),
        template::Reference::Collection { .. } => return Ok(()),
    };
    let Some(rest) = path.strip_prefix("page.") else {
        return Ok(());
    };
    let keys: Vec<&str> = rest.split('.').collect();
    match keys.as_slice() {
        ["name" | "title" | "description"] if scalar => Ok(()),
        [key] | [key, _] => {
            let Some(field) = fields.get(*key) else {
                return Err(format!("`{path}`: the page has no field {key}"));
            };
            match (field.kind, keys.get(1)) {
                (FieldType::Links, Some(link)) if field.keys.iter().any(|k| k == link) => Ok(()),
                (FieldType::Links, Some(link)) => Err(format!(
                    "`{path}`: {key} has no key {link}; it has {}",
                    field.keys.join(", ")
                )),
                (FieldType::Links, None) => Err(format!(
                    "`{path}` holds several links; name one, such as page.{key}.{}",
                    field.keys.first().map_or("repo", String::as_str)
                )),
                (FieldType::List, _) => {
                    Err(format!("`{path}` is a list, which does not fit in a line"))
                }
                (_, Some(_)) => Err(format!(
                    "`{path}`: {key} is a {}, with no keys",
                    field.kind.as_str()
                )),
                (_, None) if scalar => Ok(()),
                (_, None) => Err(format!("`{path}` does not hold banner art")),
            }
        }
        _ => Err(format!("`{path}` is not a field of the page")),
    }
}

fn text_field(key: &str, value: &Value) -> Result<Value, String> {
    match value {
        Value::String(_) => Ok(value.clone()),
        _ => Err(format!("metadata.{key} must be a string; quote it")),
    }
}

/// Checks one field's value against its type.
fn field_value(key: &str, field: &FieldSpec, value: Value) -> Result<Value, String> {
    let at = format!("metadata.{key}");
    match (field.kind, &value) {
        (FieldType::String | FieldType::Url, Value::String(_)) => Ok(value),
        (FieldType::String | FieldType::Url, _) => Err(format!("{at} must be a string; quote it")),
        (FieldType::Int, Value::Number(n)) if n.is_i64() => Ok(value),
        (FieldType::Int, _) => Err(format!("{at} must be a whole number")),
        (FieldType::Bool, Value::Bool(_)) => Ok(value),
        (FieldType::Bool, _) => Err(format!("{at} must be true or false")),
        (FieldType::Enum, Value::String(s)) if field.values.contains(s) => Ok(value),
        (FieldType::Enum, _) => Err(format!("{at} must be one of {}", field.values.join(", "))),
        (FieldType::List, Value::Array(items)) if items.iter().all(Value::is_string) => Ok(value),
        (FieldType::List, _) => Err(format!("{at} must be a list of strings, such as [a, b]")),
        (FieldType::Links, Value::Object(links)) => {
            let mut kept = Map::new();
            for (link, url) in links {
                if !field.keys.contains(link) {
                    return Err(format!(
                        "{at}.{link} is not a key of {key}; it has {}",
                        field.keys.join(", ")
                    ));
                }
                match url {
                    Value::Null => {}
                    Value::String(_) => {
                        kept.insert(link.clone(), url.clone());
                    }
                    _ => return Err(format!("{at}.{link} must be a URL")),
                }
            }
            Ok(Value::Object(kept))
        }
        (FieldType::Links, _) => Err(format!("{at} must map {} to URLs", field.keys.join(", "))),
    }
}

fn field_list(fields: &indexmap::IndexMap<String, FieldSpec>) -> String {
    let mut names = vec!["name", "title", "description"];
    names.extend(fields.keys().map(String::as_str));
    names.join(", ")
}

fn check_page_spec(spec: &PageSpec, member: bool) -> Result<(), String> {
    match &spec.content {
        None if spec.style == PageStyle::Markdown => {
            return Err("spec.content is missing: give a file or inline markdown".to_owned());
        }
        None => {}
        Some(PageContent {
            file: Some(_),
            inline: Some(_),
        }) => return Err("spec.content takes a file or inline markdown, not both".to_owned()),
        Some(PageContent {
            file: None,
            inline: None,
        }) => return Err("spec.content needs a file or inline markdown".to_owned()),
        Some(PageContent {
            file: Some(file), ..
        }) => check_content_file(file)?,
        Some(PageContent {
            inline: Some(_), ..
        }) => {}
    }
    if let Some(buffer) = &spec.buffer
        && (buffer.trim().is_empty() || buffer.contains('\n'))
    {
        return Err("spec.buffer must be one line of text".to_owned());
    }
    if member && spec.style == PageStyle::Colorscheme {
        return Err("spec.style colorscheme belongs to a page under pages/".to_owned());
    }
    if member && spec.json_ld.is_some() {
        return Err(
            "spec.json_ld belongs to pages under pages/; a collection's pages take their type's"
                .to_owned(),
        );
    }
    Ok(())
}

/// A content file sits beside its manifest: a plain `.md` file name.
fn check_content_file(file: &str) -> Result<(), String> {
    let valid = file.len() > 3
        && file.ends_with(".md")
        && !file.starts_with('.')
        && !file.contains("..")
        && file
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "spec.content.file {file:?} must be a .md file beside the manifest, named [a-z0-9._-]"
        ))
    }
}

fn check_nav(spec: &NavSpec) -> Result<(), String> {
    let mut sections = std::collections::HashSet::new();
    for section in &spec.sections {
        let at = format!("section {}", section.name);
        if !is_valid_name(&section.name) {
            return Err(format!("{at}: a name must match [a-z0-9][a-z0-9-]*"));
        }
        if !sections.insert(&section.name) {
            return Err(format!("{at} appears twice"));
        }
        if let Some(title) = &section.title {
            check_nav_text(title).map_err(|err| format!("{at}: title: {err}"))?;
        }
        for (index, item) in section.items.iter().enumerate() {
            check_nav_item(item).map_err(|err| format!("{at}, item {}: {err}", index + 1))?;
        }
    }
    if let Some(count) = &spec.statusline.count
        && !is_valid_name(count)
    {
        return Err(format!(
            "statusline.count {count:?} is not a collection name"
        ));
    }
    Ok(())
}

fn check_nav_item(item: &crate::spec::NavItem) -> Result<(), String> {
    let set = [
        item.page.is_some(),
        item.collection.is_some(),
        item.command.is_some(),
        item.link.is_some(),
    ];
    if set.iter().filter(|set| **set).count() != 1 {
        return Err("give exactly one of page, collection, command and link".to_owned());
    }
    if item.open_max.is_some() && item.collection.is_none() {
        return Err("open_max goes with a collection".to_owned());
    }
    for name in [&item.page, &item.collection].into_iter().flatten() {
        if !is_valid_name(name) {
            return Err(format!("{name:?} is not a name"));
        }
    }
    match &item.link {
        Some(NavLink::Path(path)) => {
            template::value_path(path)?;
            if path.starts_with("page.") {
                return Err(format!("`{path}`: the nav belongs to no page"));
            }
            if let Some(label) = &item.label {
                check_nav_text(label).map_err(|err| format!("label: {err}"))?;
            }
        }
        Some(NavLink::Literal(link)) => {
            if item.label.is_some() {
                return Err("a spelled-out link carries its own label".to_owned());
            }
            check_nav_text(&link.label).map_err(|err| format!("label: {err}"))?;
            check_nav_text(&link.url).map_err(|err| format!("url: {err}"))?;
        }
        None if item.label.is_some() => return Err("label goes with a link".to_owned()),
        None => {}
    }
    Ok(())
}

fn check_nav_text(text: &str) -> Result<(), String> {
    for reference in template::check_text(text)? {
        if let template::Reference::Value(path) = reference
            && path.starts_with("page.")
        {
            return Err(format!("`{path}`: the nav belongs to no page"));
        }
    }
    Ok(())
}

fn is_kind_name(kind: &str) -> bool {
    kind.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && kind.chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_field_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn check_page_kind(spec: &PageKindSpec) -> Result<(), String> {
    let names = &spec.names;
    if !is_kind_name(&names.kind) {
        return Err(format!(
            "names.kind {:?} must be a CamelCase word such as Project",
            names.kind
        ));
    }
    if Kind::from_declared(&names.kind).is_some() || names.kind.ends_with("List") {
        return Err(format!(
            "names.kind {} is taken: built-in kinds and names ending in List are",
            names.kind
        ));
    }
    let nouns = [&names.singular, &names.plural]
        .into_iter()
        .chain(&names.short_names);
    for noun in nouns {
        if !noun.chars().next().is_some_and(|c| c.is_ascii_lowercase()) || !is_valid_name(noun) {
            return Err(format!("{noun:?} must match [a-z][a-z0-9-]*"));
        }
        if BUILTIN_RESOURCES.contains(&noun.as_str()) {
            return Err(format!("{noun:?} names a built-in resource"));
        }
    }
    for (name, field) in &spec.fields {
        check_field(name, field).map_err(|err| format!("fields.{name}: {err}"))?;
    }
    if let Some(summary) = &spec.summary {
        match spec.fields.get(summary) {
            Some(field) if field.kind == FieldType::String => {}
            Some(_) => return Err(format!("summary {summary} must be a string field")),
            None => return Err(format!("summary {summary} is not a field")),
        }
    }
    if let Some(footer) = &spec.footer {
        check_content(footer, &spec.fields)
            .map_err(|problems| format!("footer {}", problems.join("; ")))?;
    }
    if let Some(json_ld) = &spec.json_ld {
        if !is_kind_name(&json_ld.kind) {
            return Err(format!(
                "json_ld.type {:?} must be a schema.org type such as SoftwareSourceCode",
                json_ld.kind
            ));
        }
        for (key, source) in &json_ld.map {
            if key.is_empty() || key.starts_with('@') || key == "name" || key == "url" {
                return Err(format!(
                    "json_ld.map.{key}: @context, @type, name and url are filled by the site"
                ));
            }
            if !json_ld_source(source, &spec.fields) {
                return Err(format!(
                    "json_ld.map.{key}: {source} is not title, description, a field or a link of one"
                ));
            }
        }
    }
    Ok(())
}

/// Whether `source` names something of a page: `title`, `description`, a
/// field, or one key of a `links` field.
pub fn json_ld_source(source: &str, fields: &indexmap::IndexMap<String, FieldSpec>) -> bool {
    match source.split_once('.') {
        None => matches!(source, "title" | "description") || fields.contains_key(source),
        Some((field, key)) => fields
            .get(field)
            .is_some_and(|f| f.kind == FieldType::Links && f.keys.iter().any(|k| k == key)),
    }
}

fn check_field(name: &str, field: &FieldSpec) -> Result<(), String> {
    if !is_field_name(name) {
        return Err("a field name must match [a-z][a-z0-9_]*".to_owned());
    }
    if matches!(name, "name" | "title" | "description") {
        return Err(format!("{name} is declared by every page"));
    }
    let is = |kind| field.kind == kind;
    let enum_attributes = !field.values.is_empty() || !field.ok.is_empty();
    if enum_attributes && !is(FieldType::Enum) {
        return Err("values and ok belong to enum fields".to_owned());
    }
    if !is(FieldType::Links) && !field.keys.is_empty() {
        return Err("keys belong to links fields".to_owned());
    }
    if is(FieldType::Enum) {
        if field.values.is_empty() {
            return Err("an enum lists its values".to_owned());
        }
        if let Some(stray) = field.ok.iter().find(|ok| !field.values.contains(ok)) {
            return Err(format!("ok value {stray} is not among the values"));
        }
        if has_duplicates(&field.values) {
            return Err("values repeat".to_owned());
        }
    }
    if is(FieldType::Links) {
        if field.keys.is_empty() {
            return Err("a links field lists its keys".to_owned());
        }
        if field.keys.iter().any(|key| !is_field_name(key)) || has_duplicates(&field.keys) {
            return Err("keys must be distinct names matching [a-z][a-z0-9_]*".to_owned());
        }
    }
    Ok(())
}

fn has_duplicates(items: &[String]) -> bool {
    let mut seen = std::collections::HashSet::new();
    !items.iter().all(|item| seen.insert(item))
}

fn check_collection(name: &str, spec: &CollectionSpec) -> Result<(), String> {
    if RESERVED_NAMES.contains(&name) || RESERVED_COLLECTIONS.contains(&name) {
        return Err(format!(
            "{name} is reserved: a collection cannot be named {} or {}",
            RESERVED_NAMES.join(", "),
            RESERVED_COLLECTIONS.join(", ")
        ));
    }
    if !is_kind_name(&spec.kind) {
        return Err(format!(
            "kind {:?} must name a page type such as Project",
            spec.kind
        ));
    }
    for column in &spec.columns {
        if column.width == Some(0) {
            return Err(format!("columns: {} has no width", column.field));
        }
    }
    Ok(())
}

/// Checks a collection against its page type: the fields it orders, badges
/// and shows must exist and fit.
pub fn check_collection_fields(spec: &CollectionSpec, kind: &PageKindSpec) -> Result<(), String> {
    let field = |name: &str| kind.fields.get(name);
    if let Some(order) = &spec.order {
        match field(order).map(|f| f.kind) {
            Some(FieldType::Int | FieldType::String) => {}
            Some(other) => {
                return Err(format!(
                    "order {order} is a {} field; order by an int or a string",
                    other.as_str()
                ));
            }
            None => {
                return Err(format!(
                    "order {order} is not a field of {}",
                    kind.names.kind
                ));
            }
        }
    }
    if let Some(badge) = &spec.badge {
        match field(badge) {
            Some(f) if f.kind == FieldType::Enum && !f.ok.is_empty() => {}
            Some(_) => {
                return Err(format!(
                    "badge {badge} must be an enum field with ok values"
                ));
            }
            None => {
                return Err(format!(
                    "badge {badge} is not a field of {}",
                    kind.names.kind
                ));
            }
        }
    }
    for column in &spec.columns {
        match field(&column.field) {
            Some(f) if f.kind != FieldType::Links => {}
            Some(_) => {
                return Err(format!(
                    "column {}: links do not fit a column",
                    column.field
                ));
            }
            None => {
                return Err(format!(
                    "column {} is not a field of {}",
                    column.field, kind.names.kind
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{self, SITE, THEME};

    fn registry() -> Registry {
        let kind = parse(
            Path::new("kinds/project.yaml"),
            testing::PROJECT_KIND.as_bytes(),
            &Registry::default(),
        )
        .unwrap();
        let collection = parse(
            Path::new("collections/projects.yaml"),
            testing::PROJECTS.as_bytes(),
            &Registry::default(),
        )
        .unwrap();
        let (Manifest::PageKind { spec: kind, .. }, Manifest::Collection { name, spec }) =
            (kind, collection)
        else {
            panic!("not a page kind and a collection");
        };
        Registry::new([kind], [(name, spec)])
    }

    fn project(source: &str) -> Result<Manifest, Error> {
        parse(
            Path::new("projects/atlas.yaml"),
            source.as_bytes(),
            &registry(),
        )
    }

    fn page_of(manifest: Manifest) -> Page {
        match manifest {
            Manifest::Page(page) => page,
            other => panic!("not a page: {other:?}"),
        }
    }

    #[test]
    fn locates_the_layout() {
        let at = |p: &str| locate(Path::new(p)).unwrap();
        let manifest = |kind, name: Option<&str>| {
            Some(Location::Manifest {
                kind,
                name: name.map(str::to_owned),
            })
        };
        assert_eq!(at("site.yaml"), manifest(Kind::Site, None));
        assert_eq!(at("profile.yaml"), manifest(Kind::Profile, None));
        assert_eq!(at("nav.yaml"), manifest(Kind::Nav, None));
        assert_eq!(at("themes/nord.yaml"), manifest(Kind::Theme, Some("nord")));
        assert_eq!(
            at("kinds/project.yaml"),
            manifest(Kind::PageKind, Some("project"))
        );
        assert_eq!(
            at("collections/projects.yaml"),
            manifest(Kind::Collection, Some("projects"))
        );
        assert_eq!(at("pages/about.yaml"), manifest(Kind::Page, Some("about")));
        assert_eq!(
            at("projects/atlas.yaml"),
            Some(Location::Member {
                collection: "projects".to_owned(),
                name: "atlas".to_owned()
            })
        );
        assert_eq!(
            at("pages/about.md"),
            Some(Location::Content {
                dir: "pages".to_owned(),
                file: "about.md".to_owned()
            })
        );
        assert!(at("themes/nord.yml").is_none());
        assert!(at("themes/notes.md").is_none());
        assert!(at("README.md").is_none());
        assert!(at("projects/notes.txt").is_none());
        assert!(at("Docs/x.yaml").is_none());
        assert!(locate(Path::new("projects/Atlas.yaml")).is_err());
        assert!(locate(Path::new("profile.md")).is_err());
        assert!(locate(Path::new("../site.yaml")).is_err());
        assert!(locate(Path::new("/site.yaml")).is_err());
    }

    #[test]
    fn path_inverts_locate() {
        for kind in Kind::ALL {
            let name = (!kind.is_singleton()).then_some("x");
            let located = locate(Path::new(&path(kind, name))).unwrap().unwrap();
            assert_eq!(
                located,
                Location::Manifest {
                    kind,
                    name: name.map(str::to_owned)
                }
            );
        }
    }

    #[test]
    fn parses_a_member_against_its_type() {
        let page = page_of(project(testing::ATLAS).unwrap());
        assert_eq!(page.kind, "Project");
        assert_eq!(page.name, "atlas");
        assert_eq!(page.collection.as_deref(), Some("projects"));
        assert_eq!(page.title(), "Atlas");
        assert_eq!(page.metadata["highlight"], 1);
        assert_eq!(page.metadata["tags"], json!(["rust", "aerospace"]));
        assert_eq!(
            page.metadata["links"],
            json!({"repo": "https://github.com/example/atlas"})
        );
        assert_eq!(page.route().as_deref(), Some("/projects/atlas"));
        assert_eq!(page.buffer(), "projects/atlas.md");
        assert_eq!(page.content_path().as_deref(), Some("projects/atlas.md"));
        assert!(!page.metadata.contains_key("name"));
    }

    #[test]
    fn member_fields_are_typed() {
        for (from, to, expected) in [
            (
                "status: active",
                "status: done",
                "must be one of active, wip, archived",
            ),
            ("highlight: 1", "highlight: first", "must be a whole number"),
            (
                "tags: [rust, aerospace]",
                "tags: rust",
                "must be a list of strings",
            ),
            (
                "  repo:",
                "  rep:",
                "metadata.links.rep is not a key of links",
            ),
            (
                "tagline:",
                "tagln:",
                "metadata.tagln is not a field of Project",
            ),
            ("  title: Atlas\n", "", "metadata.title is missing"),
            (
                "  tagline: CubeSat ADCS in Rust\n",
                "",
                "metadata.tagline is missing",
            ),
            ("name: atlas", "name: other", "metadata.name is other"),
            ("kind: Project", "kind: Post", "declares kind Post"),
        ] {
            let source = testing::ATLAS.replace(from, to);
            assert_ne!(source, testing::ATLAS, "{from}");
            let err = project(&source).unwrap_err().to_string();
            assert!(err.contains(expected), "{from} -> {to}: {err}");
            assert!(err.contains("projects/atlas.yaml"), "{err}");
        }
    }

    #[test]
    fn empty_values_are_left_out() {
        let page =
            page_of(project(&testing::ATLAS.replace("highlight: 1", "highlight: ~")).unwrap());
        assert!(!page.metadata.contains_key("highlight"));
    }

    #[test]
    fn a_member_needs_its_collection_and_type() {
        let err = parse(
            Path::new("posts/hello.yaml"),
            testing::ATLAS.as_bytes(),
            &registry(),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("declare it in collections/posts.yaml"),
            "{err}"
        );

        let orphaned = Registry::new(
            [],
            [(
                "projects".to_owned(),
                match parse(
                    Path::new("collections/projects.yaml"),
                    testing::PROJECTS.as_bytes(),
                    &Registry::default(),
                )
                .unwrap()
                {
                    Manifest::Collection { spec, .. } => spec,
                    _ => unreachable!(),
                },
            )],
        );
        let err = parse(
            Path::new("projects/atlas.yaml"),
            testing::ATLAS.as_bytes(),
            &orphaned,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("no page type in kinds/ declares"),
            "{err}"
        );
    }

    #[test]
    fn parses_a_page() {
        let page = page_of(
            parse(
                Path::new("pages/about.yaml"),
                testing::ABOUT.as_bytes(),
                &Registry::default(),
            )
            .unwrap(),
        );
        assert_eq!(page.kind, "Page");
        assert_eq!(page.collection, None);
        assert_eq!(page.route().as_deref(), Some("/about"));
        assert_eq!(page.buffer(), "about.md");
        assert_eq!(page.description(), Some("{{ profile.headline }}"));

        let index = page_of(
            parse(
                Path::new("pages/index.yaml"),
                testing::INDEX.as_bytes(),
                &Registry::default(),
            )
            .unwrap(),
        );
        assert_eq!(index.route().as_deref(), Some("/"));
        assert_eq!(index.buffer(), "README.md");
    }

    #[test]
    fn pages_are_checked() {
        let about = |from: &str, to: &str| {
            let source = testing::ABOUT.replace(from, to);
            assert_ne!(source, testing::ABOUT, "{from}");
            parse(
                Path::new("pages/about.yaml"),
                source.as_bytes(),
                &Registry::default(),
            )
            .unwrap_err()
            .to_string()
        };
        for (from, to, expected) in [
            (
                "  title: about\n",
                "  title: about\n  tagline: x\n",
                "declares only name, title and description",
            ),
            (
                "{{ profile.headline }}",
                "{{ profile.age }}",
                "the profile has name",
            ),
            (
                "{{ profile.headline }}",
                "{{ page.tagline }}",
                "the page has no field tagline",
            ),
            (
                "    file: about.md",
                "    file: ../about.md",
                "must be a .md file beside the manifest",
            ),
            (
                "    file: about.md",
                "    file: about.txt",
                "must be a .md file",
            ),
            (
                "  content:\n    file: about.md\n",
                "",
                "spec.content is missing",
            ),
            (
                "    file: about.md",
                "    file: about.md\n    inline: x",
                "not both",
            ),
            ("  json_ld: person", "  json_ld: robot", "json_ld"),
        ] {
            let err = about(from, to);
            assert!(err.contains(expected), "{to}: {err}");
        }
        let inline = testing::NOT_FOUND.replace("{{ link / README.md }}", "{{ nope }}");
        let err = parse(
            Path::new("pages/not-found.yaml"),
            inline.as_bytes(),
            &Registry::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("spec.content.inline line 1: unknown directive `nope`"),
            "{err}"
        );
    }

    #[test]
    fn colorscheme_pages_need_no_content() {
        let theme =
            "kind: Page\nmetadata:\n  name: theme\n  title: theme\nspec:\n  style: colorscheme\n";
        let page = page_of(
            parse(
                Path::new("pages/theme.yaml"),
                theme.as_bytes(),
                &Registry::default(),
            )
            .unwrap(),
        );
        assert_eq!(page.spec.style, PageStyle::Colorscheme);
    }

    #[test]
    fn manifests_round_trip() {
        for (path, text) in [
            ("site.yaml", SITE),
            ("profile.yaml", testing::PROFILE),
            ("nav.yaml", testing::NAV),
            ("themes/nord.yaml", THEME),
            ("kinds/project.yaml", testing::PROJECT_KIND),
            ("collections/projects.yaml", testing::PROJECTS),
            ("pages/about.yaml", testing::ABOUT),
            ("projects/atlas.yaml", testing::ATLAS),
        ] {
            let first = parse(Path::new(path), text.as_bytes(), &registry()).unwrap();
            let rendered = render(&first).unwrap();
            let again = parse(Path::new(path), rendered.as_bytes(), &registry())
                .unwrap_or_else(|err| panic!("{path}: {err}\n{rendered}"));
            assert_eq!(first, again, "{path}\n{rendered}");
        }
    }

    #[test]
    fn site_values_are_free() {
        let Manifest::Site(site) = parse(
            Path::new("site.yaml"),
            SITE.as_bytes(),
            &Registry::default(),
        )
        .unwrap() else {
            panic!("not a site");
        };
        assert_eq!(site.values["banner"]["alt"], "EX");
        assert_eq!(site.values["about"], "Design to platform.");
    }

    #[test]
    fn singletons_have_no_metadata_and_named_kinds_a_matching_name() {
        let err = parse(
            Path::new("site.yaml"),
            SITE.replace("spec:", "metadata:\n  name: x\nspec:")
                .as_bytes(),
            &Registry::default(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("unknown field `metadata`"),
            "{err}"
        );

        let err = parse(
            Path::new("themes/dawn.yaml"),
            THEME.as_bytes(),
            &Registry::default(),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("metadata.name is nord, but the file names it dawn"),
            "{err}"
        );
    }

    #[test]
    fn rejects_colors_that_are_not_hex() {
        let hostile = THEME.replace("\"#88c0d0\"", "\"red;}</style><script>\"");
        let err = parse(
            Path::new("themes/nord.yaml"),
            hostile.as_bytes(),
            &Registry::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("#rrggbb"), "{err}");
    }

    #[test]
    fn site_theme_must_be_a_name() {
        let err = parse(
            Path::new("site.yaml"),
            SITE.replace("theme: nord", "theme: ../x").as_bytes(),
            &Registry::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("name must match"), "{err}");
    }

    #[test]
    fn reads_the_declared_kind() {
        assert_eq!(declared_kind(testing::ATLAS).as_deref(), Some("Project"));
        assert_eq!(declared_kind(THEME).as_deref(), Some("Theme"));
        assert_eq!(declared_kind("title: no kind\n"), None);
        assert_eq!(declared_kind("kind: [\n"), None);
    }

    #[test]
    fn rejects_a_kind_that_disagrees_with_the_path() {
        let err = parse(
            Path::new("site.yaml"),
            THEME.as_bytes(),
            &Registry::default(),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("declares kind Theme, but this path holds a Site"),
            "{err}"
        );
    }

    #[test]
    fn rejects_unknown_fields() {
        let err = parse(
            Path::new("themes/nord.yaml"),
            THEME.replace("dark:", "drak:").as_bytes(),
            &Registry::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("themes/nord.yaml"), "{err}");
        assert!(err.to_string().contains("unknown field `drak`"), "{err}");
    }

    #[test]
    fn rejects_files_outside_the_layout() {
        let err = parse(
            Path::new("notes.yaml"),
            SITE.as_bytes(),
            &Registry::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not a content file"), "{err}");
        let err = parse(
            Path::new("pages/about.md"),
            b"# about\n",
            &Registry::default(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("a markdown file is content"),
            "{err}"
        );
    }

    #[test]
    fn nav_items_are_checked() {
        let nav = |item: &str| {
            let source = format!(
                "kind: Nav\nspec:\n  sections:\n    - name: files\n      items:\n        - {item}\n"
            );
            parse(
                Path::new("nav.yaml"),
                source.as_bytes(),
                &Registry::default(),
            )
        };
        assert!(nav("page: index").is_ok());
        assert!(nav("{collection: projects, open_max: 8}").is_ok());
        assert!(nav("command: find").is_ok());
        assert!(nav("{link: profile.email, label: mail}").is_ok());
        assert!(nav("link: {label: x, url: \"https://x.test\"}").is_ok());
        for (item, expected) in [
            ("{page: index, command: help}", "exactly one"),
            (
                "{page: index, open_max: 2}",
                "open_max goes with a collection",
            ),
            ("command: fly", "unknown variant"),
            ("link: profile.age", "the profile has name"),
            ("link: page.title", "belongs to no page"),
            (
                "{link: {label: x, url: \"https://x.test\"}, label: z}",
                "carries its own label",
            ),
            ("page: About", "is not a name"),
        ] {
            let err = nav(item).unwrap_err().to_string();
            assert!(err.contains(expected), "{item}: {err}");
        }
    }

    #[test]
    fn page_kinds_are_checked() {
        let kind = |from: &str, to: &str| {
            let source = testing::PROJECT_KIND.replace(from, to);
            assert_ne!(source, testing::PROJECT_KIND, "{from}");
            parse(
                Path::new("kinds/project.yaml"),
                source.as_bytes(),
                &Registry::default(),
            )
            .unwrap_err()
            .to_string()
        };
        for (from, to, expected) in [
            ("kind: Project", "kind: Theme", "is taken"),
            ("kind: Project", "kind: project", "CamelCase"),
            (
                "plural: projects",
                "plural: pages",
                "names a built-in resource",
            ),
            (
                "summary: tagline",
                "summary: status",
                "must be a string field",
            ),
            ("summary: tagline", "summary: nope", "is not a field"),
            (
                "ok: [active]",
                "ok: [done]",
                "ok value done is not among the values",
            ),
            ("keys: [repo, docs, demo]", "keys: []", "lists its keys"),
            (
                "{{ clone page.links.repo }}",
                "{{ clone page.links.home }}",
                "links has no key home",
            ),
            (
                "{{ clone page.links.repo }}",
                "{{ clone page.tags }}",
                "is a list",
            ),
            (
                "codeRepository: links.repo",
                "codeRepository: links.home",
                "json_ld.map.codeRepository",
            ),
            ("tagline:", "title:", "is declared by every page"),
        ] {
            let err = kind(from, to);
            assert!(err.contains(expected), "{to}: {err}");
        }
    }

    #[test]
    fn collections_are_checked() {
        let err = parse(
            Path::new("collections/theme.yaml"),
            testing::PROJECTS
                .replace("name: projects", "name: theme")
                .as_bytes(),
            &Registry::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("theme is reserved"), "{err}");

        let Manifest::Collection { spec, .. } = parse(
            Path::new("collections/projects.yaml"),
            testing::PROJECTS.as_bytes(),
            &Registry::default(),
        )
        .unwrap() else {
            panic!("not a collection");
        };
        let kind = registry().kind("Project").unwrap().clone();
        check_collection_fields(&spec, &kind).unwrap();
        let mut bad = spec.clone();
        bad.order = Some("tags".to_owned());
        assert!(
            check_collection_fields(&bad, &kind)
                .unwrap_err()
                .contains("order tags is a list")
        );
        let mut bad = spec.clone();
        bad.badge = Some("tagline".to_owned());
        assert!(
            check_collection_fields(&bad, &kind)
                .unwrap_err()
                .contains("must be an enum")
        );
        let mut bad = spec;
        bad.columns[0].field = "links".to_owned();
        assert!(
            check_collection_fields(&bad, &kind)
                .unwrap_err()
                .contains("links do not fit")
        );
    }
}
