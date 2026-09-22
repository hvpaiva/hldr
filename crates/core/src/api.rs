//! Wire format of the private API, shared by the server and the CLI.
//!
//! Every resource travels in the same envelope as its file, `kind`,
//! `metadata` and `spec`, plus the timestamps the server keeps and a
//! `status`, so the CLI handles any kind the same way. Page types defined in
//! content are resources too: discovery and schemas are built from their
//! PageKinds, so `hldr` learns them without a release.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::manifest::Kind;
use crate::spec::{
    CollectionSpec, FieldSpec, FieldType, NavSpec, PageKindSpec, PageSpec, ProfileSpec, SiteSpec,
    ThemeSpec,
};

/// A resource as the API serves it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Resource<M, S> {
    /// Resource type, such as `Theme` or `Project`.
    pub kind: String,
    /// The file's metadata, and the timestamps the server keeps.
    pub metadata: M,
    /// What the file declares under `spec`.
    pub spec: S,
    /// Derived state. Never accepted on write.
    pub status: Value,
}

/// A collection of resources of one kind.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct List<T> {
    /// List type, such as `ThemeList`.
    pub kind: String,
    pub items: Vec<T>,
}

/// Bookkeeping of a singleton, which has no name.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SingletonMetadata {
    /// When the file last changed, ISO-8601 UTC. Set by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// Name and bookkeeping of a named resource.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NamedMetadata {
    /// Identifier: the file's name, which the file repeats here.
    pub name: String,
    /// When the file last changed, ISO-8601 UTC. Set by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// Site settings as the API serves them.
pub type Site = Resource<SingletonMetadata, SiteSpec>;

/// Profile as the API serves it.
pub type Profile = Resource<SingletonMetadata, ProfileSpec>;

/// The file tree and statusline as the API serves them.
pub type Nav = Resource<SingletonMetadata, NavSpec>;

/// A page type as the API serves it.
pub type PageKind = Resource<NamedMetadata, PageKindSpec>;

/// A collection as the API serves it.
pub type Collection = Resource<NamedMetadata, CollectionSpec>;

/// Color scheme as the API serves it.
pub type Theme = Resource<NamedMetadata, ThemeSpec>;

/// A page of any type as the API serves it: its metadata holds `name`,
/// `title`, `description` and its type's fields, then the timestamps.
pub type Page = Resource<Map<String, Value>, PageSpec>;

/// What one pass of the indexer changed, by kind.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncReport {
    /// Counts by the kind files declare, such as `Theme` or `Project`.
    pub kinds: BTreeMap<String, KindReport>,
}

/// What one pass changed for one kind.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KindReport {
    /// Resources created or rewritten.
    #[serde(default)]
    pub upserted: u32,
    /// Resources whose files did not change.
    #[serde(default)]
    pub skipped: u32,
    /// Resources whose files are gone.
    #[serde(default)]
    pub deleted: u32,
}

impl SyncReport {
    /// Counts a resource the pass wrote (`true`) or left as it was.
    pub fn count(&mut self, kind: &str, written: bool) {
        let entry = self.kinds.entry(kind.to_owned()).or_default();
        if written {
            entry.upserted += 1;
        } else {
            entry.skipped += 1;
        }
    }

    pub fn deleted(&mut self, kind: &str, count: u32) {
        if count > 0 {
            self.kinds.entry(kind.to_owned()).or_default().deleted += count;
        }
    }

    pub fn get(&self, kind: &str) -> KindReport {
        self.kinds.get(kind).copied().unwrap_or_default()
    }
}

/// State of the content sync, as `hldr sync status` shows it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SyncStatus {
    /// Always `SyncStatus`.
    pub kind: String,
    /// Where content comes from, such as `github.com/hvpaiva/hldr-content@main`.
    pub source: String,
    /// GitHub repository as `owner/name`; null for content read from a
    /// directory. Where `hldr` writes.
    #[serde(default)]
    pub repository: Option<String>,
    /// Branch or tag the server follows in `repository`.
    #[serde(default)]
    pub branch: Option<String>,
    /// Content commit being served; null for content read from a directory.
    pub revision: Option<String>,
    /// When the served content was materialized, ISO-8601 UTC.
    pub synced_at: Option<String>,
    /// When a sync last ran, successful or not, ISO-8601 UTC.
    pub last_attempt_at: Option<String>,
    /// Why the last attempt failed; null when it succeeded.
    pub last_error: Option<String>,
    /// What this sync changed. Present only on the response to a sync that
    /// indexed content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<SyncReport>,
}

#[cfg(feature = "store")]
impl SyncStatus {
    pub fn new(
        source: String,
        origin: Option<(String, String)>,
        state: crate::SyncState,
        report: Option<SyncReport>,
    ) -> Self {
        let (repository, branch) = origin.unzip();
        Self {
            kind: "SyncStatus".to_owned(),
            source,
            repository,
            branch,
            revision: state.revision,
            synced_at: state.synced_at,
            last_attempt_at: state.last_attempt_at,
            last_error: state.last_error,
            report,
        }
    }
}

/// Whether an event reports normal operation or something to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum EventType {
    Normal,
    Warning,
}

impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Warning => "Warning",
        }
    }
}

/// Something that happened to the server, as `hldr get events` lists it.
/// Kept 30 days; the same event repeated in a row is one event with a count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    /// Always `Event`.
    pub kind: String,
    pub metadata: EventMetadata,
    #[serde(rename = "type")]
    pub event_type: EventType,
    /// What happened, in one word, such as `Started`, `Synced` or
    /// `SyncFailed`.
    pub reason: String,
    pub message: String,
    /// Content commit served when it happened; null for content read from a
    /// directory, and before the first sync.
    pub revision: Option<String>,
    /// How many times in a row it happened.
    pub count: u32,
    /// When it first happened, ISO-8601 UTC.
    pub first_at: String,
    /// When it last happened, ISO-8601 UTC.
    pub last_at: String,
}

/// An event's identity and place in the stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventMetadata {
    /// Identifier, a number that never changes.
    pub name: String,
    /// Position in the stream, raised whenever the event is written again:
    /// `GET /api/v1/events?after=SEQ` answers what changed after it.
    pub seq: i64,
}

/// Events oldest first, and the stream position they were read at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventList {
    /// Always `EventList`.
    pub kind: String,
    /// Where the stream stood when the list was read: the `after` of the
    /// next call that wants only what is new.
    pub seq: i64,
    pub items: Vec<Event>,
}

/// The running server, as `hldr describe server` shows it. Only on the
/// private API: nothing here is public.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Server {
    /// Always `Server`.
    pub kind: String,
    pub metadata: SingletonMetadata,
    pub status: ServerStatus,
}

/// What the server observes about itself.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServerStatus {
    /// Release version of the binary.
    pub version: String,
    /// Git commit the binary was built from.
    pub revision: String,
    /// When this process started, ISO-8601 UTC.
    pub started_at: String,
    /// How long it has run, such as `3d4h` or `12m5s`.
    pub uptime: String,
    pub content: ContentState,
    pub database: DatabaseState,
    /// The GitHub rate limit as its last answer reported it; null before the
    /// first request, and for content read from a directory.
    pub github: Option<GitHubQuota>,
}

/// Where the served content stands.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContentState {
    /// Such as `github.com/hvpaiva/hldr-content@main`.
    pub source: String,
    /// Content commit being served; null for a directory.
    pub revision: Option<String>,
    /// When it was materialized, ISO-8601 UTC.
    pub synced_at: Option<String>,
    /// When a sync last ran, successful or not, ISO-8601 UTC.
    pub last_attempt_at: Option<String>,
    /// Why the last attempt failed; null when it succeeded.
    pub last_error: Option<String>,
}

/// What the database takes on disk.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseState {
    /// The main file, in bytes.
    pub bytes: u64,
    /// The write-ahead log not yet folded into the main file, in bytes.
    pub wal_bytes: u64,
}

/// A GitHub rate limit, from the `x-ratelimit-*` headers of its answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitHubQuota {
    /// Requests allowed in the window: 60 an hour without a token.
    pub limit: u64,
    /// Requests left in the window.
    pub remaining: u64,
    /// When the window starts over, ISO-8601 UTC.
    pub reset_at: String,
    /// When GitHub reported this, ISO-8601 UTC.
    pub observed_at: String,
}

/// How far back `/api/v1/metrics/*` reads: the last N UTC days, today
/// included, or every day kept. Written `7d`, `30d` or `all`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Since {
    Days(u16),
    All,
}

impl Since {
    /// The longest period in days: ten years.
    pub const MAX_DAYS: u16 = 3660;
}

impl Default for Since {
    fn default() -> Self {
        Self::Days(7)
    }
}

impl std::str::FromStr for Since {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == "all" {
            return Ok(Self::All);
        }
        text.strip_suffix('d')
            .and_then(|days| days.parse().ok())
            .filter(|days| (1..=Self::MAX_DAYS).contains(days))
            .map(Self::Days)
            .ok_or_else(|| {
                format!(
                    "{text:?} is not a period: use a number of days such as 7d, up to {}d, or all",
                    Self::MAX_DAYS
                )
            })
    }
}

impl std::fmt::Display for Since {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Days(days) => write!(f, "{days}d"),
            Self::All => f.write_str("all"),
        }
    }
}

/// The UTC days a metrics answer covers, both ends included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Period {
    /// As asked for: `7d`, `30d` or `all`.
    pub since: String,
    /// First day, `YYYY-MM-DD`.
    pub from: String,
    /// Last day, today.
    pub to: String,
}

/// Visits per day, oldest first, one row for every day of the period.
///
/// A visitor is known only within its UTC day, so visitors over several
/// days are the sum of each day's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DailyMetrics {
    /// Always `DailyMetrics`.
    pub kind: String,
    pub period: Period,
    pub total: Visits,
    pub items: Vec<DayVisits>,
}

/// Counts over a period.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Visits {
    /// Pages served, 404s aside.
    pub views: u64,
    /// Distinct visitors, summed day by day.
    pub visitors: u64,
    /// Requests from user agents that declare themselves bots.
    pub bots: u64,
}

/// One UTC day's counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DayVisits {
    /// `YYYY-MM-DD`.
    pub day: String,
    #[serde(flatten)]
    pub visits: Visits,
}

/// Views per route over a period, most viewed first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageMetrics {
    /// Always `PageMetrics`.
    pub kind: String,
    pub period: Period,
    pub items: Vec<PageVisits>,
}

/// One route's counts. Every request answered 404 is counted under `404`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageVisits {
    pub route: String,
    pub views: u64,
    /// Distinct visitors, summed day by day.
    pub visitors: u64,
}

/// Views per referring host over a period, most first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReferrerMetrics {
    /// Always `ReferrerMetrics`.
    pub kind: String,
    pub period: Period,
    pub items: Vec<ReferrerVisits>,
}

/// Views that came from one host, lowercased; the site itself is left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReferrerVisits {
    pub host: String,
    pub views: u64,
}

/// Body of `POST /api/v1/sync`. Empty, it syncs whatever the branch points
/// at; with a revision, it waits for the branch to point there first, since
/// GitHub can serve a stale ref for a few seconds after a push.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncRequest {
    /// Commit the caller just pushed, as 40 hex digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// Error body, RFC 9457 (`application/problem+json`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Problem {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
}

/// A resource type the server exposes, as `hldr api-resources` lists it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApiResource {
    /// Plural name used in paths, such as `themes`.
    pub name: String,
    /// Singular name, used by `hldr explain`.
    pub singular: String,
    pub short_names: Vec<String>,
    /// Key into the document served by [`schemas`].
    pub kind: String,
    /// True when exactly one resource of this kind exists and it has no name.
    pub singleton: bool,
    /// Verbs the CLI supports on this resource.
    pub verbs: Vec<String>,
    /// Where the resource lives in the content repository.
    pub source: Source,
    /// Columns of the table `hldr get` prints.
    pub columns: Vec<Column>,
}

/// The manifest that declares a resource. A page's markdown sits beside it,
/// under the name its `spec.content.file` gives.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Source {
    /// Path under the content root; `{name}` stands for the resource name.
    /// Empty for what the server keeps itself, such as events, which no
    /// file declares and no verb writes.
    pub path: String,
}

/// A table column, in the style of kubectl's printer columns.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Column {
    /// Header text.
    pub name: String,
    /// Field shown, as a kubectl-style JSONPath such as `.spec.title`.
    pub json_path: String,
    /// Shown only with `-o wide`.
    pub wide: bool,
    /// For an enum field: the values it takes, which the CLI colors as a
    /// status, those not in `ok` as a warning.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// For an enum field: the values drawn as good, as the site draws them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ok: Vec<String>,
}

const READ_VERBS: &[&str] = &["get", "describe", "explain"];
const WRITE_VERBS: &[&str] = &["edit", "apply", "diff", "patch"];

/// A page type as discovery sees it: its spec and the collection that is its
/// home.
pub struct PageType<'a> {
    pub collection: &'a str,
    pub spec: &'a PageKindSpec,
}

/// Resource types the server exposes, in display order: the built-in kinds,
/// then one per page type.
pub fn resources(types: &[PageType<'_>]) -> List<ApiResource> {
    let updated = ("UPDATED", ".metadata.updated_at", true);
    let mut items = vec![
        builtin(
            Kind::Site,
            "site",
            &[],
            &[
                ("TITLE", ".spec.title", false),
                ("THEME", ".spec.theme", false),
                updated,
            ],
        ),
        builtin(
            Kind::Profile,
            "profile",
            &[],
            &[
                ("NAME", ".spec.name", false),
                ("HEADLINE", ".spec.headline", false),
                ("EMAIL", ".spec.email", true),
                updated,
            ],
        ),
        builtin(
            Kind::Nav,
            "nav",
            &[],
            &[("COUNT", ".spec.statusline.count", false), updated],
        ),
        builtin(
            Kind::PageKind,
            "pagekinds",
            &["pk"],
            &[
                ("NAME", ".metadata.name", false),
                ("KIND", ".spec.names.kind", false),
                ("PLURAL", ".spec.names.plural", false),
                ("SUMMARY", ".spec.summary", true),
                updated,
            ],
        ),
        builtin(
            Kind::Collection,
            "collections",
            &["col"],
            &[
                ("NAME", ".metadata.name", false),
                ("KIND", ".spec.kind", false),
                ("ENABLED", ".spec.enabled", false),
                ("ORDER", ".spec.order", true),
                updated,
            ],
        ),
        builtin(
            Kind::Page,
            "pages",
            &[],
            &[
                ("NAME", ".metadata.name", false),
                ("TITLE", ".metadata.title", false),
                ("DRAFT", ".spec.draft", false),
                ("BUFFER", ".spec.buffer", true),
                updated,
            ],
        ),
        builtin(
            Kind::Theme,
            "themes",
            &["th"],
            &[
                ("NAME", ".metadata.name", false),
                ("TITLE", ".spec.title", false),
                ("DARK", ".spec.dark", false),
                ("HIDDEN", ".spec.hidden", false),
                ("ORDER", ".spec.order", true),
                ("BG", ".spec.colors.bg", true),
                ("ACCENT", ".spec.colors.accent", true),
                updated,
            ],
        ),
        events(),
        kept(
            "server",
            "Server",
            &[],
            true,
            &["get", "describe", "explain"],
            &[
                ("VERSION", ".status.version", false),
                ("UPTIME", ".status.uptime", false),
                ("CONTENT", ".status.content.revision", false),
                ("SYNCED", ".status.content.synced_at", false),
                ("STARTED", ".status.started_at", true),
                ("DB BYTES", ".status.database.bytes", true),
                ("GITHUB LEFT", ".status.github.remaining", true),
            ],
        ),
    ];
    let mut types: Vec<&PageType<'_>> = types.iter().collect();
    types.sort_by(|a, b| a.spec.names.plural.cmp(&b.spec.names.plural));
    items.extend(types.into_iter().map(page_type));
    List {
        kind: "APIResourceList".to_owned(),
        items,
    }
}

fn verbs(singleton: bool) -> Vec<String> {
    let mut verbs: Vec<String> = READ_VERBS
        .iter()
        .chain(WRITE_VERBS)
        .map(|&verb| verb.to_owned())
        .collect();
    if !singleton {
        verbs.push("delete".to_owned());
    }
    verbs
}

fn column(name: &str, json_path: &str, wide: bool) -> Column {
    Column {
        name: name.to_owned(),
        json_path: json_path.to_owned(),
        wide,
        ..Column::default()
    }
}

fn columns(columns: &[(&str, &str, bool)]) -> Vec<Column> {
    columns
        .iter()
        .map(|&(name, json_path, wide)| column(name, json_path, wide))
        .collect()
}

fn builtin(
    kind: Kind,
    name: &str,
    short_names: &[&str],
    printed: &[(&str, &str, bool)],
) -> ApiResource {
    ApiResource {
        name: name.to_owned(),
        singular: kind.as_str().to_lowercase(),
        short_names: short_names.iter().map(|&s| s.to_owned()).collect(),
        kind: kind.as_str().to_owned(),
        singleton: kind.is_singleton(),
        verbs: verbs(kind.is_singleton()),
        source: Source {
            path: kind.path_template().to_owned(),
        },
        columns: columns(printed),
    }
}

/// Events, their type colored as a status is: `Normal` good, `Warning` not.
fn events() -> ApiResource {
    let mut events = kept(
        "events",
        "Event",
        &["ev"],
        false,
        &["get", "explain", "watch"],
        &[
            ("LAST SEEN", ".last_at", false),
            ("TYPE", ".type", false),
            ("REASON", ".reason", false),
            ("COUNT", ".count", false),
            ("MESSAGE", ".message", false),
            ("FIRST SEEN", ".first_at", true),
            ("REVISION", ".revision", true),
        ],
    );
    for column in events.columns.iter_mut().filter(|c| c.name == "TYPE") {
        column.values = [EventType::Normal, EventType::Warning]
            .map(|kind| kind.as_str().to_owned())
            .to_vec();
        column.ok = vec![EventType::Normal.as_str().to_owned()];
    }
    events
}

/// A resource the server keeps itself: read-only, with no file behind it.
fn kept(
    name: &str,
    kind: &str,
    short_names: &[&str],
    singleton: bool,
    verbs: &[&str],
    printed: &[(&str, &str, bool)],
) -> ApiResource {
    ApiResource {
        name: name.to_owned(),
        singular: kind.to_lowercase(),
        short_names: short_names.iter().map(|&s| s.to_owned()).collect(),
        kind: kind.to_owned(),
        singleton,
        verbs: verbs.iter().map(|&verb| verb.to_owned()).collect(),
        source: Source {
            path: String::new(),
        },
        columns: columns(printed),
    }
}

/// A page type's resource. Its columns come from its fields: short scalars
/// in the table, text and lists with `-o wide`. An enum's column carries
/// its values, so the CLI colors them as the site does.
fn page_type(page_type: &PageType<'_>) -> ApiResource {
    let names = &page_type.spec.names;
    let mut printed = vec![column("NAME", ".metadata.name", false)];
    let field = |name: &String, spec: &FieldSpec, wide| Column {
        values: spec.values.clone(),
        ok: spec.ok.clone(),
        ..column(&name.to_uppercase(), &format!(".metadata.{name}"), wide)
    };
    for (name, spec) in &page_type.spec.fields {
        if matches!(
            spec.kind,
            FieldType::Enum | FieldType::Int | FieldType::Bool
        ) {
            printed.push(field(name, spec, false));
        }
    }
    printed.push(column("DRAFT", ".spec.draft", false));
    printed.push(column("TITLE", ".metadata.title", true));
    for (name, spec) in &page_type.spec.fields {
        if matches!(
            spec.kind,
            FieldType::String | FieldType::Url | FieldType::List
        ) {
            printed.push(field(name, spec, true));
        }
    }
    printed.push(column("UPDATED", ".metadata.updated_at", true));
    ApiResource {
        name: names.plural.clone(),
        singular: names.singular.clone(),
        short_names: names.short_names.clone(),
        kind: names.kind.clone(),
        singleton: false,
        verbs: verbs(false),
        source: Source {
            path: format!("{}/{{name}}.yaml", page_type.collection),
        },
        columns: printed,
    }
}

/// JSON Schema of every resource kind, keyed by kind: the built-in kinds
/// from their types, each page type from its fields.
pub fn schemas(types: &[PageType<'_>]) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("Site".to_owned(), titled::<Site>("Site"));
    out.insert("Profile".to_owned(), titled::<Profile>("Profile"));
    out.insert("Nav".to_owned(), titled::<Nav>("Nav"));
    out.insert("PageKind".to_owned(), titled::<PageKind>("PageKind"));
    out.insert("Collection".to_owned(), titled::<Collection>("Collection"));
    out.insert("Theme".to_owned(), titled::<Theme>("Theme"));
    out.insert("Event".to_owned(), titled::<Event>("Event"));
    out.insert("Server".to_owned(), titled::<Server>("Server"));
    out.insert(
        "Page".to_owned(),
        page_schema("Page", &IndexMap::new(), "A page outside any collection."),
    );
    for page_type in types {
        let names = &page_type.spec.names;
        let about = format!(
            "A page of the {} type, which kinds/ declares; its pages live in {}/.",
            names.singular, page_type.collection
        );
        out.insert(
            names.kind.clone(),
            page_schema(&names.kind, &page_type.spec.fields, &about),
        );
    }
    out
}

fn titled<T: JsonSchema>(title: &str) -> Value {
    let mut schema = schema_for!(T);
    schema.insert("title".to_owned(), title.into());
    schema.to_value()
}

/// A page resource's schema, in the shape schemars gives the built-in
/// kinds: `$ref`s into `$defs`, enums as `oneOf` of `const`s.
fn page_schema(kind: &str, fields: &IndexMap<String, FieldSpec>, about: &str) -> Value {
    let spec = schema_for!(PageSpec).to_value();
    let mut defs = spec
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut page_spec = spec;
    if let Some(object) = page_spec.as_object_mut() {
        object.remove("$defs");
        object.remove("$schema");
        object.remove("title");
    }
    defs.insert("PageSpec".to_owned(), page_spec);

    let string = |description: &str| json!({ "type": "string", "description": description });
    let mut properties = Map::new();
    properties.insert(
        "name".to_owned(),
        string("Identifier: the file's name, which the file repeats here."),
    );
    properties.insert(
        "title".to_owned(),
        string("Display name, also the HTML title. Templates allowed."),
    );
    properties.insert(
        "description".to_owned(),
        string("Meta description of the page. Templates allowed."),
    );
    let mut required = vec![json!("name"), json!("title")];
    for (name, field) in fields {
        properties.insert(name.clone(), field_schema(field));
        if field.required {
            required.push(json!(name));
        }
    }
    properties.insert(
        "created_at".to_owned(),
        string("When the page was first indexed, ISO-8601 UTC. Set by the server."),
    );
    properties.insert(
        "updated_at".to_owned(),
        string(
            "When the page's manifest or content last changed, ISO-8601 UTC. Set by the server.",
        ),
    );
    let metadata = format!("{kind}Metadata");
    defs.insert(
        metadata.clone(),
        json!({
            "type": "object",
            "description": "What the page declares about itself, which its frontmatter shows.",
            "properties": properties,
            "required": required,
        }),
    );
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": kind,
        "description": about,
        "type": "object",
        "properties": {
            "kind": { "type": "string", "description": "Resource type, such as `Theme` or `Project`." },
            "metadata": { "$ref": format!("#/$defs/{metadata}"), "description": "The file's metadata, and the timestamps the server keeps." },
            "spec": { "$ref": "#/$defs/PageSpec", "description": "How the page is built, the same for every page type." },
            "status": { "description": "Derived state. Never accepted on write." },
        },
        "required": ["kind", "metadata", "spec", "status"],
        "$defs": defs,
    })
}

fn field_schema(field: &FieldSpec) -> Value {
    let mut schema = match field.kind {
        FieldType::String => json!({ "type": "string" }),
        FieldType::Url => json!({ "type": "string", "format": "uri" }),
        FieldType::Int => json!({ "type": "integer" }),
        FieldType::Bool => json!({ "type": "boolean" }),
        FieldType::List => json!({ "type": "array", "items": { "type": "string" } }),
        FieldType::Enum => {
            let variants: Vec<Value> = field
                .values
                .iter()
                .map(|value| {
                    let mut variant = json!({ "const": value, "type": "string" });
                    if field.ok.contains(value) {
                        variant["description"] =
                            json!("Drawn as good: in the ok color, and as the tree badge.");
                    }
                    variant
                })
                .collect();
            json!({ "oneOf": variants })
        }
        FieldType::Links => {
            let keys: Map<String, Value> = field
                .keys
                .iter()
                .map(|key| {
                    (
                        key.clone(),
                        json!({ "type": "string", "description": "A URL; linked when it is safe." }),
                    )
                })
                .collect();
            json!({ "type": "object", "properties": keys, "additionalProperties": false })
        }
    };
    if let Some(description) = &field.description {
        schema["description"] = json!(description);
    }
    schema
}

#[cfg(feature = "store")]
mod from_store {
    use super::*;

    fn empty_status() -> Value {
        Value::Object(Map::new())
    }

    fn singleton<S>(kind: Kind, spec: S, updated_at: &str) -> Resource<SingletonMetadata, S> {
        Resource {
            kind: kind.as_str().to_owned(),
            metadata: SingletonMetadata {
                updated_at: Some(updated_at.to_owned()),
            },
            spec,
            status: empty_status(),
        }
    }

    fn named<S>(kind: Kind, name: &str, spec: S, updated_at: &str) -> Resource<NamedMetadata, S> {
        Resource {
            kind: kind.as_str().to_owned(),
            metadata: NamedMetadata {
                name: name.to_owned(),
                updated_at: Some(updated_at.to_owned()),
            },
            spec,
            status: empty_status(),
        }
    }

    impl From<&crate::store::Site> for Site {
        fn from(item: &crate::store::Site) -> Self {
            singleton(Kind::Site, item.spec.clone(), &item.updated_at)
        }
    }

    impl From<&crate::store::Profile> for Profile {
        fn from(item: &crate::store::Profile) -> Self {
            singleton(Kind::Profile, item.spec.clone(), &item.updated_at)
        }
    }

    impl From<&crate::store::Nav> for Nav {
        fn from(item: &crate::store::Nav) -> Self {
            singleton(Kind::Nav, item.spec.clone(), &item.updated_at)
        }
    }

    impl From<&crate::store::PageKind> for PageKind {
        fn from(item: &crate::store::PageKind) -> Self {
            named(
                Kind::PageKind,
                &item.name,
                item.spec.clone(),
                &item.updated_at,
            )
        }
    }

    impl From<&crate::store::Collection> for Collection {
        fn from(item: &crate::store::Collection) -> Self {
            named(
                Kind::Collection,
                &item.name,
                item.spec.clone(),
                &item.updated_at,
            )
        }
    }

    impl From<&crate::store::Theme> for Theme {
        fn from(item: &crate::store::Theme) -> Self {
            let spec = ThemeSpec {
                title: item.title.clone(),
                dark: item.dark,
                order: item.order,
                hidden: item.hidden,
                colors: item.colors.clone(),
            };
            named(Kind::Theme, &item.slug, spec, &item.updated_at)
        }
    }

    impl From<&crate::store::Page> for Page {
        fn from(item: &crate::store::Page) -> Self {
            let mut metadata = Map::new();
            metadata.insert("name".to_owned(), Value::String(item.page.name.clone()));
            metadata.extend(item.page.metadata.clone());
            metadata.insert(
                "created_at".to_owned(),
                Value::String(item.created_at.clone()),
            );
            metadata.insert(
                "updated_at".to_owned(),
                Value::String(item.updated_at.clone()),
            );
            Resource {
                kind: item.page.kind.clone(),
                metadata,
                spec: item.page.spec.clone(),
                status: empty_status(),
            }
        }
    }

    /// Resources as a `<Kind>List`.
    pub fn list<'a, T: 'a, R: From<&'a T>>(
        kind: &str,
        items: impl IntoIterator<Item = &'a T>,
    ) -> List<R> {
        List {
            kind: format!("{kind}List"),
            items: items.into_iter().map(R::from).collect(),
        }
    }
}

#[cfg(feature = "store")]
pub use from_store::list;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{self, Manifest, Registry};
    use crate::testing;
    use std::path::Path;

    fn field<'a>(schema: &'a Value, path: &[&str]) -> &'a Value {
        let defs = &schema["$defs"];
        let mut node = schema;
        for name in path {
            node = &node["properties"][*name];
            // An `Option<T>` is `anyOf [T, null]`, as `hldr explain` reads it.
            if let Some(some) = node["anyOf"]
                .as_array()
                .and_then(|variants| variants.iter().find(|v| v["type"] != "null"))
            {
                node = some;
            }
            if let Some(reference) = node["$ref"].as_str() {
                let def = reference.trim_start_matches("#/$defs/");
                node = &defs[def];
            }
        }
        node
    }

    #[test]
    fn a_period_is_days_or_all() {
        assert_eq!("7d".parse(), Ok(Since::Days(7)));
        assert_eq!("3660d".parse(), Ok(Since::Days(3660)));
        assert_eq!("all".parse(), Ok(Since::All));
        for bad in ["", "d", "0d", "7", "3661d", "-1d", "7w", "ALL"] {
            assert!(bad.parse::<Since>().is_err(), "{bad:?}");
        }
        assert_eq!(Since::default().to_string(), "7d");
        assert_eq!(Since::All.to_string(), "all");
    }

    fn project_kind() -> PageKindSpec {
        match manifest::parse(
            Path::new("kinds/project.yaml"),
            testing::PROJECT_KIND.as_bytes(),
            &Registry::default(),
        )
        .unwrap()
        {
            Manifest::PageKind { spec, .. } => spec,
            _ => unreachable!(),
        }
    }

    fn types(spec: &PageKindSpec) -> Vec<PageType<'_>> {
        vec![PageType {
            collection: "projects",
            spec,
        }]
    }

    #[test]
    fn field_docs_reach_the_schema() {
        let kind = project_kind();
        let schemas = schemas(&types(&kind));
        let theme = &schemas["Theme"];
        assert_eq!(theme["title"], "Theme");
        let hidden = field(theme, &["spec", "hidden"]);
        assert!(
            hidden["description"]
                .as_str()
                .unwrap()
                .starts_with("Left out of the picker")
        );

        let repo = field(&schemas["Profile"], &["spec", "links", "github"]);
        assert_eq!(repo["description"], "GitHub profile URL.");

        let project = &schemas["Project"];
        assert_eq!(project["title"], "Project");
        let status = field(project, &["metadata", "status"]);
        assert_eq!(status["oneOf"][0]["const"], "active");
        assert!(status["oneOf"][0]["description"].is_string());
        let frontmatter = field(project, &["spec", "frontmatter"]);
        assert!(
            frontmatter["description"]
                .as_str()
                .unwrap()
                .contains("frontmatter")
        );
        let required = field(project, &["metadata"])["required"].clone();
        assert_eq!(required, json!(["name", "title", "tagline", "status"]));
    }

    #[test]
    fn every_resource_has_a_schema() {
        let kind = project_kind();
        let schemas = schemas(&types(&kind));
        let catalog = resources(&types(&kind));
        assert_eq!(catalog.items.len(), schemas.len());
        for resource in &catalog.items {
            assert!(schemas.contains_key(&resource.kind), "{}", resource.kind);
        }
        let projects = catalog.items.iter().find(|r| r.name == "projects").unwrap();
        assert_eq!(projects.source.path, "projects/{name}.yaml");
        assert_eq!(projects.short_names, ["p"]);
    }

    #[test]
    fn every_column_names_a_schema_field() {
        let kind = project_kind();
        let schemas = schemas(&types(&kind));
        for resource in resources(&types(&kind)).items {
            let schema = &schemas[&resource.kind];
            for column in &resource.columns {
                let path: Vec<&str> = column
                    .json_path
                    .trim_start_matches('.')
                    .split('.')
                    .collect();
                assert!(
                    field(schema, &path).is_object(),
                    "{}: {}",
                    resource.kind,
                    column.json_path
                );
            }
        }
    }

    #[test]
    fn enum_columns_carry_their_values() {
        let kind = project_kind();
        let catalog = resources(&types(&kind));
        let projects = catalog.items.iter().find(|r| r.name == "projects").unwrap();
        let status = projects
            .columns
            .iter()
            .find(|c| c.name == "STATUS")
            .unwrap();
        assert_eq!(status.values, ["active", "wip", "archived"]);
        assert_eq!(status.ok, ["active"]);
        let draft = projects.columns.iter().find(|c| c.name == "DRAFT").unwrap();
        assert!(draft.values.is_empty() && draft.ok.is_empty());
        let json = serde_json::to_value(draft).unwrap();
        assert!(json.get("values").is_none() && json.get("ok").is_none());
    }

    #[test]
    fn only_named_kinds_can_be_deleted() {
        let kind = project_kind();
        for resource in resources(&types(&kind)).items {
            let deletable = resource.verbs.iter().any(|verb| verb == "delete");
            let from_content = !resource.source.path.is_empty();
            assert_eq!(
                deletable,
                from_content && !resource.singleton,
                "{}",
                resource.kind
            );
        }
    }

    #[test]
    fn what_the_server_keeps_is_read_only() {
        let catalog = resources(&[]);
        for name in ["events", "server"] {
            let resource = catalog.items.iter().find(|r| r.name == name).unwrap();
            assert!(resource.source.path.is_empty(), "{name}");
            for verb in WRITE_VERBS.iter().chain(&["delete"]) {
                assert!(!resource.verbs.iter().any(|v| v == verb), "{name}: {verb}");
            }
        }
        let events = catalog.items.iter().find(|r| r.name == "events").unwrap();
        assert!(events.verbs.iter().any(|v| v == "watch"));
        assert_eq!(events.short_names, ["ev"]);
    }

    /// Keys a YAML mapping holds at `indent` under the line `under:`, or at
    /// the top when `under` is empty.
    fn keys(text: &str, under: &str) -> Vec<String> {
        let mut inside = under.is_empty();
        let indent = if under.is_empty() { "" } else { "  " };
        let mut out = Vec::new();
        for line in text.lines() {
            if !under.is_empty() && !line.starts_with(' ') {
                inside = line == format!("{under}:");
                continue;
            }
            if !inside {
                continue;
            }
            let Some(rest) = line.strip_prefix(indent) else {
                continue;
            };
            if rest.starts_with([' ', '-']) {
                continue;
            }
            if let Some((key, _)) = rest.split_once(':') {
                out.push(key.to_owned());
            }
        }
        out.sort();
        out
    }

    fn required(schema: &Value, part: &str) -> Vec<String> {
        let node = field(schema, &[part]);
        let mut names: Vec<String> = node["required"]
            .as_array()
            .map(|names| {
                names
                    .iter()
                    .map(|n| n.as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// The smallest file of each kind: its required fields and nothing else.
    const MINIMAL: [(&str, &str); 8] = [
        (
            "site.yaml",
            "kind: Site\nspec:\n  title: T\n  theme: nord\n",
        ),
        (
            "profile.yaml",
            "kind: Profile\nspec:\n  name: N\n  headline: H\n  bio: B\n",
        ),
        ("nav.yaml", "kind: Nav\nspec:\n  sections: []\n"),
        ("themes/nord.yaml", testing::THEME),
        (
            "kinds/post.yaml",
            "kind: PageKind\nmetadata:\n  name: post\nspec:\n  names: {kind: Post, singular: post, plural: posts}\n",
        ),
        (
            "collections/blog.yaml",
            "kind: Collection\nmetadata:\n  name: blog\nspec:\n  kind: Post\n",
        ),
        (
            "pages/about.yaml",
            "kind: Page\nmetadata:\n  name: about\n  title: A\nspec:\n  content:\n    inline: x\n",
        ),
        (
            "projects/atlas.yaml",
            "kind: Project\nmetadata:\n  name: atlas\n  title: T\n  tagline: L\n  status: wip\nspec:\n  content:\n    inline: x\n",
        ),
    ];

    #[test]
    fn the_schema_requires_what_the_file_requires() {
        let kind = project_kind();
        let schemas = schemas(&types(&kind));
        let collection = match manifest::parse(
            Path::new("collections/projects.yaml"),
            testing::PROJECTS.as_bytes(),
            &Registry::default(),
        )
        .unwrap()
        {
            Manifest::Collection { spec, .. } => spec,
            _ => unreachable!(),
        };
        let registry = Registry::new([kind.clone()], [("projects".to_owned(), collection)]);
        for (path, text) in MINIMAL {
            let manifest = manifest::parse(Path::new(path), text.as_bytes(), &registry)
                .unwrap_or_else(|err| panic!("{path}: {err}"));
            let schema = &schemas[manifest.kind()];
            let mut expected = required(schema, "spec");
            if matches!(manifest, Manifest::Page(_)) {
                // Required unless the style is colorscheme, which a schema
                // cannot say; the parser enforces it.
                expected.push("content".to_owned());
            }
            assert_eq!(keys(text, "spec"), expected, "{path}: spec");
            if matches!(manifest, Manifest::Page(_)) {
                assert_eq!(
                    keys(text, "metadata"),
                    required(schema, "metadata"),
                    "{path}: metadata"
                );
            }
            for key in keys(text, "spec") {
                let missing = text.replace(&format!("\n  {key}:"), "\n  missing_on_purpose:");
                assert!(
                    manifest::parse(Path::new(path), missing.as_bytes(), &registry).is_err(),
                    "{path} parses without spec.{key}"
                );
            }
        }
    }

    #[test]
    fn reports_count_by_kind() {
        let mut report = SyncReport::default();
        report.count("Theme", true);
        report.count("Theme", false);
        report.deleted("Project", 2);
        report.deleted("Page", 0);
        assert_eq!(
            report.get("Theme"),
            KindReport {
                upserted: 1,
                skipped: 1,
                deleted: 0
            }
        );
        assert_eq!(report.get("Project").deleted, 2);
        assert!(!report.kinds.contains_key("Page"));
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["kinds"]["Theme"]["upserted"], 1);
    }
}
