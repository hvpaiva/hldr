//! What each kind of content file declares under `spec`, shared by the
//! parser, the indexer and the wire format. Doc comments here are what
//! `hldr explain` prints, through [`crate::api::schemas`].

use std::collections::BTreeMap;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::types::{Palette, ProfileLinks};

/// Global settings, from `site.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SiteSpec {
    /// Site name, shown atop the file tree and after every page title.
    pub title: String,
    /// Theme a visitor sees before picking one; the name of a `Theme`.
    pub theme: String,
    /// Free values that templates read as `{{ site.values.<key> }}`. A value
    /// more than one page shows, or one a page shares with the code, such as
    /// the `banner` and the `intro` line, lives here.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub values: Map<String, Value>,
}

/// Who the site is about, from `profile.yaml`. Typed, because the JSON-LD
/// `Person`, the `links` directive and the nav read these exact fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileSpec {
    /// Full name.
    pub name: String,
    /// Short line under the name.
    pub headline: String,
    /// Paragraph that introduces the profile.
    pub bio: String,
    /// Public contact address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "ProfileLinks::is_empty")]
    pub links: ProfileLinks,
}

/// The file tree beside every page, and the statusline under it, from
/// `nav.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavSpec {
    /// Sections of the tree, top to bottom.
    pub sections: Vec<NavSection>,
    #[serde(default, skip_serializing_if = "Statusline::is_empty")]
    pub statusline: Statusline,
}

/// A section of the tree, which a visitor can fold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavSection {
    /// Identifier, and the key the browser keeps the section's fold under:
    /// renaming it forgets every visitor's fold.
    pub name: String,
    /// Heading of the section; its name when absent. Templates allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Rows, top to bottom.
    pub items: Vec<NavItem>,
}

/// A row of the tree: exactly one of `page`, `collection`, `command` and
/// `link`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavItem {
    /// A page by name, shown as its buffer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// A collection by name: a folder of its pages, or `off` while disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    /// With `collection`: folded above this many pages, unless the visitor
    /// is inside it. Always open when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_max: Option<usize>,
    /// One of the commands the site has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<NavCommand>,
    /// A link out: a value path such as `profile.links.github`, or a fixed
    /// `{label, url}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<NavLink>,
    /// With a value path `link`: the text shown, instead of the path's last
    /// key. Templates allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A command of the site. The set is fixed; the nav picks which show and
/// in what order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NavCommand {
    /// The page whose style is `colorscheme`.
    Colorscheme,
    /// Find a file by name; needs JavaScript.
    Find,
    /// The manual.
    Help,
    /// Server version, content revision and sync state.
    Checkhealth,
}

impl NavCommand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Colorscheme => "colorscheme",
            Self::Find => "find",
            Self::Help => "help",
            Self::Checkhealth => "checkhealth",
        }
    }
}

/// Where a nav link points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum NavLink {
    /// A value path, such as `profile.links.github` or `profile.email`. A URL
    /// shows with `↗`, an email address with `@`; links from `profile.links`
    /// carry `rel="me"`.
    Path(String),
    /// A link spelled out.
    Literal(LiteralLink),
}

/// A link spelled out in the nav.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LiteralLink {
    /// Text shown. Templates allowed.
    pub label: String,
    /// Where it goes. Templates allowed.
    pub url: String,
}

/// What the statusline shows beside the version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Statusline {
    /// A collection whose published pages the statusline counts, such as
    /// `projects`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<String>,
}

impl Statusline {
    pub fn is_empty(&self) -> bool {
        self.count.is_none()
    }
}

/// A page type, from `kinds/<name>.yaml`, in the manner of a Kubernetes
/// CRD: its pages declare `kind: <names.kind>` and live in its Collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageKindSpec {
    pub names: KindNames,
    /// What its pages declare in `metadata` beside `name`, `title` and
    /// `description`, in the order their frontmatter shows it.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub fields: IndexMap<String, FieldSpec>,
    /// A string field that sums a page up: its meta description, its second
    /// line in listings, and its JSON-LD description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Markdown with directives, appended to every page of the type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<String>,
    /// Structured data for search engines, filled from each page's fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_ld: Option<JsonLdSpec>,
}

/// How a page type is named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KindNames {
    /// What its pages declare as `kind`, such as `Project`.
    pub kind: String,
    /// One of its pages, such as `project`, as `hldr explain` takes it.
    pub singular: String,
    /// The API resource and the CLI noun, such as `projects`.
    pub plural: String,
    /// Other names the CLI accepts, such as `p`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub short_names: Vec<String>,
}

/// A field of a page type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    #[serde(rename = "type")]
    pub kind: FieldType,
    /// Every page of the type must declare it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    /// With `enum`: the values accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// With `enum`: the values drawn as good, in the ok color and as the
    /// tree badge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ok: Vec<String>,
    /// With `links`: the keys accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    /// What the field means, as `hldr explain` prints it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// What a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    /// Text.
    String,
    /// A whole number.
    Int,
    /// true or false.
    Bool,
    /// One of `values`.
    Enum,
    /// A list of strings.
    List,
    /// A URL, linked when it is safe.
    Url,
    /// URLs under the keys `keys` names; unsafe ones are not linked.
    Links,
}

impl FieldType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Int => "int",
            Self::Bool => "bool",
            Self::Enum => "enum",
            Self::List => "list",
            Self::Url => "url",
            Self::Links => "links",
        }
    }

    /// Whether a value of this type prints on one line of text.
    pub fn is_scalar(self) -> bool {
        !matches!(self, Self::List | Self::Links)
    }
}

/// Structured data of a page type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JsonLdSpec {
    /// A schema.org type, such as `SoftwareSourceCode`.
    #[serde(rename = "type")]
    pub kind: String,
    /// JSON-LD keys and the fields that fill them, such as `codeRepository:
    /// links.repo`. `name` and `url` come from the page; `description` is
    /// the summary unless mapped.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub map: BTreeMap<String, String>,
}

/// The home of one page type, from `collections/<name>.yaml`: its listing
/// at `/<name>`, and its pages in `<name>/`, served at `/<name>/<page>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionSpec {
    /// The page type it holds, as its pages declare it, such as `Project`.
    pub kind: String,
    /// Whether it is listed and served. While false, its listing and pages
    /// answer 404 and the tree shows it `off`.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Line atop the listing, also its meta description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Field its pages are ordered by, ascending; pages without it come
    /// last, then the most recently changed first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    /// An `enum` field with `ok` values: a page holding one shows it as a
    /// badge in the tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    /// Columns of the listing, after the number and the name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<ListingColumn>,
}

/// A column of a collection's listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListingColumn {
    /// The field shown.
    pub field: String,
    /// Characters the column takes; values are not padded when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<usize>,
    /// Header text; the field's name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
}

/// How a page is built, the same for every page type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageSpec {
    /// Where the markdown comes from. Required unless the style is
    /// `colorscheme`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<PageContent>,
    /// Label in the tab, the tree and the statusline; `<name>.md` by
    /// default, `<collection>/<name>.md` for a collection's page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffer: Option<String>,
    #[serde(default, skip_serializing_if = "PageStyle::is_default")]
    pub style: PageStyle,
    #[serde(default, skip_serializing_if = "FrontmatterMode::is_default")]
    pub frontmatter: FrontmatterMode,
    /// Structured data of a page outside any collection. A collection's page
    /// takes its type's `json_ld`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_ld: Option<PageJsonLd>,
    /// The HTML title is the page's title alone, without the site's after it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub standalone_title: bool,
    /// Kept off the site while true: it answers 404 and leaves listings, the
    /// tree and the sitemap. The API still serves it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub draft: bool,
}

/// Where a page's markdown is: exactly one of `file` and `inline`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageContent {
    /// A markdown file beside the manifest, such as `about.md`, without a
    /// frontmatter: the fields live in the manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The markdown itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<String>,
}

/// How a page's buffer is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PageStyle {
    /// Markdown as its source, markers visible.
    #[default]
    Markdown,
    /// The color scheme picker, under the page's markdown if it has any. At
    /// most one page has this style.
    Colorscheme,
}

impl PageStyle {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Whether a page shows its metadata atop the buffer, as a frontmatter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FrontmatterMode {
    /// Not shown.
    #[default]
    Hidden,
    /// Shown between `---` lines.
    Open,
    /// Shown as a closed fold, which opens without JavaScript.
    Folded,
}

impl FrontmatterMode {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Structured data of a page outside any collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PageJsonLd {
    /// A schema.org `Person`, from the profile.
    Person,
}

/// A color scheme, from `themes/<name>.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThemeSpec {
    /// Display name in the picker.
    pub title: String,
    /// Whether the palette is dark; sets the page's `color-scheme`.
    pub dark: bool,
    /// Place in the picker: themes with one come first, lowest first, then
    /// the others by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i64>,
    /// Left out of the picker and of `:colo` completion; `/theme/<name>`
    /// still sets it. The default theme cannot be hidden.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    pub colors: Palette,
}

fn is_false(value: &bool) -> bool {
    !value
}

fn yes() -> bool {
    true
}
