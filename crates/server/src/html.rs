//! The site as an editor: a file tree, one buffer per route, a statusline.
//! Every page is complete without JavaScript; `keys.js` and `vim.js` only
//! add the keyboard layer on top.
//!
//! What a page holds comes from content: `nav.yaml` draws the tree, pages
//! and page types draw their buffers. What stays here is how each thing is
//! drawn, the manual and the health report.

use std::collections::BTreeMap;

use hldr_core::SyncState;
use hldr_core::spec::{
    FieldType, FrontmatterMode, ListingColumn, NavCommand, NavItem, NavLink, PageJsonLd,
    PageKindSpec, PageStyle, ProfileSpec,
};
use hldr_core::store::{Collection, Page, Theme};
use hldr_core::template::{self, Block};
use maud::{DOCTYPE, Markup, PreEscaped, html};
use serde_json::Value;

use crate::Site;
use crate::model::Model;
use crate::source::{self, Expand, safe_url};
use crate::theme;

/// Commands that only exist once vim.js can run them.
const JS_ONLY: [&str; 1] = ["find"];
/// Where the server's source lives, for links to its releases and commits.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// What a buffer needs around it.
struct Frame<'a> {
    site: &'a Site,
    model: &'a Model,
    theme: &'a Theme,
    path: &'a str,
    file: &'a str,
    filetype: &'a str,
    title: &'a str,
    description: &'a str,
    json_ld: Option<String>,
}

impl Model {
    /// A page title under the site's name.
    fn under_site(&self, title: &str) -> String {
        format!("{title} · {}", self.title())
    }
}

fn layout(frame: Frame<'_>, lines: Vec<Markup>) -> Markup {
    let canonical = format!("{}{}", frame.site.origin, frame.path);
    let description = oneline(frame.description);
    let tab = frame
        .file
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(frame.file);
    let tab = if frame.file.ends_with('/') {
        format!("{tab}/")
    } else {
        tab.to_owned()
    };
    html! {
        (DOCTYPE)
        html lang="en" data-theme=(frame.theme.slug) {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover";
                title { (frame.title) }
                meta name="description" content=(description);
                link rel="canonical" href=(canonical);
                meta property="og:title" content=(frame.title);
                meta property="og:description" content=(description);
                meta property="og:url" content=(canonical);
                meta property="og:type" content="website";
                meta name="twitter:card" content="summary";
                link rel="icon" href=(format!("/favicon.svg?t={}", frame.theme.slug)) type="image/svg+xml";
                link rel="stylesheet" href=(format!("/style.css?v={}", hldr_core::VERSION));
                style { (PreEscaped(theme::root_css(frame.theme))) }
                @if let Some(json_ld) = &frame.json_ld {
                    script type="application/ld+json" { (PreEscaped(json_ld)) }
                }
            }
            body data-path=(frame.path) data-file=(frame.file) data-ft=(frame.filetype) {
                a.skip-link href="#win" { "skip to content" }
                input #tt .tt type="checkbox" aria-label="toggle the file tree";
                div #ed .ed {
                    nav #tree .tree aria-label="files" { (tree(frame.model, frame.path)) }
                    div.tabs {
                        label.tgl for="tt" title="toggle the tree (space e)" { "≡" }
                        div #bufs .bufs {
                            span.tab.on { a href=(frame.path) aria-current="page" { (tab) } }
                        }
                    }
                    main #win .win tabindex="-1" {
                        article.buf data-path=(frame.file) data-ft=(frame.filetype) {
                            @for line in &lines { (line) }
                        }
                    }
                    footer #status .status {
                        span #mode .mode { "NORMAL" }
                        span.file { label for="tt" { (frame.file) } }
                        span.ro { "[RO]" }
                        span.fill {}
                        span #showcmd .showcmd aria-hidden="true" {}
                        span.chip { a href="/help" { ":help" } }
                        span.ft { (frame.filetype) }
                        span.ver {
                            @if let Some((href, count)) = statusline_count(frame.model) {
                                a href=(href) { (count) }
                                " · "
                            }
                            a href="/health" title=":checkhealth" { "hldr " (hldr_core::VERSION) }
                        }
                        span #pos .pos { (lines.len()) "L" }
                    }
                }
                script src=(format!("/keys.js?v={}", hldr_core::VERSION)) defer {}
            }
        }
    }
}

/// `N projects` for the collection the nav counts, while it is enabled.
fn statusline_count(model: &Model) -> Option<(String, String)> {
    let name = model.nav.spec.statusline.count.as_deref()?;
    let collection = model.collection(name).filter(|c| c.spec.enabled)?;
    let kind = model.kind_of(collection)?;
    let count = model.members(collection).len();
    let noun = if count == 1 {
        &kind.names.singular
    } else {
        &kind.names.plural
    };
    Some((format!("/{name}"), format!("{count} {noun}")))
}

fn tree(model: &Model, current: &str) -> Markup {
    let context = model.context(None);
    html! {
        @for section in &model.nav.spec.sections {
            @let title = section
                .title
                .as_deref()
                .map_or_else(|| section.name.clone(), |title| model.expand(title, None));
            details.sec data-fold=(section.name) open {
                summary.hd { (title) }
                @for item in &section.items { (tree_item(model, &context, item, current)) }
            }
        }
    }
}

fn tree_item(model: &Model, context: &Value, item: &NavItem, current: &str) -> Markup {
    if let Some(name) = &item.page {
        return match model.page(name) {
            Some(page) => tree_file(
                &page.page.route().unwrap_or_default(),
                &page.page.buffer(),
                current,
            ),
            None => html! {},
        };
    }
    if let Some(name) = &item.collection {
        return tree_collection(model, name, item.open_max, current);
    }
    if let Some(command) = item.command {
        return tree_command(model, command, current);
    }
    match &item.link {
        Some(NavLink::Path(path)) => tree_value_link(model, context, path, item.label.as_deref()),
        Some(NavLink::Literal(link)) => {
            let url = model.expand(&link.url, None);
            let label = model.expand(&link.label, None);
            let icon = if url.trim().starts_with("mailto:") {
                "@"
            } else {
                "↗"
            };
            match safe_url(&url) {
                Some(url) => html! { div.row { span.ic { (icon) } a href=(url) { (label) } } },
                None => html! {},
            }
        }
        None => html! {},
    }
}

/// A link whose address is a value: `↗` for a URL, `@` for an email
/// address. Links from the profile's links are the site owner's own, so
/// they carry `rel="me"`.
fn tree_value_link(model: &Model, context: &Value, path: &str, label: Option<&str>) -> Markup {
    let value = template::text(template::lookup(context, path));
    let label = label.map_or_else(
        || path.rsplit('.').next().unwrap_or(path).to_owned(),
        |label| model.expand(label, None),
    );
    let email = path == "profile.email" || (!value.contains(':') && value.contains('@'));
    if value.is_empty() {
        return html! {};
    }
    if email {
        return html! { div.row { span.ic { "@" } a href=(format!("mailto:{value}")) { (label) } } };
    }
    let me = path.starts_with("profile.links.").then_some("me");
    match safe_url(&value) {
        Some(url) => html! { div.row { span.ic { "↗" } a href=(url) rel=[me] { (label) } } },
        None => html! {},
    }
}

fn tree_file(href: &str, label: &str, current: &str) -> Markup {
    html! {
        div.row class=[(current == href).then_some("cur")] {
            span.ic { "·" }
            a href=(href) data-file=(label) { (label) }
        }
    }
}

fn tree_collection(model: &Model, name: &str, open_max: Option<usize>, current: &str) -> Markup {
    let Some(collection) = model.collection(name) else {
        return html! {};
    };
    if !collection.spec.enabled {
        return html! {
            div.row.dir.off { span.ic { "▸" } span.grow { (name) "/" } span.badge.off { "off" } }
        };
    }
    let route = format!("/{name}");
    let members = model.members(collection);
    let inside = current == route || current.starts_with(&format!("{route}/"));
    let open = inside || open_max.is_none_or(|max| members.len() <= max);
    let kind = model.kind_of(collection);
    html! {
        details.dir data-fold=(name) open[open] {
            summary.row.dir class=[(current == route).then_some("cur")] {
                a href=(route) { (name) "/" }
                span.badge.off { (members.len()) }
            }
            div.kids {
                @for member in &members {
                    @let href = member.page.route().unwrap_or_default();
                    @let buffer = member.page.buffer();
                    div.row class=[(current == href).then_some("cur")] {
                        span.ic { "·" }
                        a href=(href) data-file=(buffer) { (last_segment(&buffer)) }
                        @if let Some(badge) = badge(collection, kind, member) { span.badge { (badge) } }
                    }
                }
            }
        }
    }
}

/// The badge field's value, when it is one of the field's good values.
fn badge<'a>(
    collection: &Collection,
    kind: Option<&PageKindSpec>,
    member: &'a Page,
) -> Option<&'a str> {
    let field = collection.spec.badge.as_deref()?;
    let value = member.page.metadata.get(field)?.as_str()?;
    let spec = kind?.fields.get(field)?;
    spec.ok.iter().any(|ok| ok == value).then_some(value)
}

fn tree_command(model: &Model, command: NavCommand, current: &str) -> Markup {
    match command {
        NavCommand::Colorscheme => {
            let href = model
                .picker()
                .and_then(|page| page.page.route())
                .unwrap_or_else(|| "/theme".to_owned());
            tree_cmd(&href, ":", "colorscheme", None, "colorscheme", current)
        }
        NavCommand::Find => {
            // Only ever seen without JavaScript, where it lists the files of
            // the first collection the tree shows.
            let href = model
                .nav
                .spec
                .sections
                .iter()
                .flat_map(|section| &section.items)
                .find_map(|item| item.collection.as_deref())
                .map_or_else(|| "/".to_owned(), |name| format!("/{name}"));
            tree_cmd(&href, "/", "find file", Some("␣ff"), "find", current)
        }
        NavCommand::Help => tree_cmd("/help", ":", "help", Some("F1"), "help", current),
        NavCommand::Checkhealth => {
            tree_cmd("/health", ":", "checkhealth", None, "checkhealth", current)
        }
    }
}

fn tree_cmd(
    href: &str,
    icon: &str,
    label: &str,
    key: Option<&str>,
    cmd: &str,
    current: &str,
) -> Markup {
    html! {
        div.row.cur[current == href].js-only[JS_ONLY.contains(&cmd)] {
            span.ic { (icon) }
            a href=(href) data-cmd=(cmd) { (label) }
            @if let Some(key) = key { span.kb { (key) } }
        }
    }
}

/// Renders the directives of one page's markdown.
struct Render<'a> {
    model: &'a Model,
    context: Value,
}

impl<'a> Render<'a> {
    fn new(model: &'a Model, page: Option<&Page>) -> Self {
        Self {
            model,
            context: model.context(page.map(|page| &page.page)),
        }
    }
}

impl Expand for Render<'_> {
    fn value(&self, path: &str) -> String {
        template::text(template::lookup(&self.context, path))
    }

    fn block(&self, block: &Block) -> Vec<Markup> {
        match block {
            Block::Banner(path) => {
                let art = template::lookup(&self.context, path);
                let text = |key: &str| art.and_then(|art| art.get(key)).and_then(Value::as_str);
                match (text("art"), text("alt")) {
                    (Some(art), Some(alt)) => {
                        vec![html! { pre.ban role="img" aria-label=(alt) { (art) } }]
                    }
                    _ => Vec::new(),
                }
            }
            Block::Collection {
                name,
                filter,
                limit,
            } => collection_rows(self.model, name, filter.as_deref(), *limit),
            Block::Links { source } => links(&self.model.profile.spec, *source),
            Block::Clone(path) => match safe_url(&self.value(path)) {
                Some(url) => vec![
                    html! { p { span.n { "git clone" } " " a href=(url) { (display_url(url)) } } },
                ],
                None => Vec::new(),
            },
        }
    }
}

/// A page of any type, in its style, with its frontmatter and its type's
/// footer.
pub fn page(
    site: &Site,
    model: &Model,
    theme: &Theme,
    page: &Page,
    content: &str,
    themes: &[Theme],
) -> Markup {
    let kind = model.kind(&page.page.kind);
    let render = Render::new(model, Some(page));
    let title = model.expand(page.page.title(), Some(&page.page));
    let mut lines = Vec::new();
    match page.page.spec.frontmatter {
        FrontmatterMode::Hidden => {}
        FrontmatterMode::Open => lines.extend(frontmatter(model, page, kind)),
        FrontmatterMode::Folded => lines.push(fold(frontmatter(model, page, kind), &title)),
    }
    let body = content.trim_start_matches(['\r', '\n']).trim_end();
    if page.page.spec.frontmatter != FrontmatterMode::Hidden && !body.is_empty() {
        lines.push(html! { p {} });
    }
    let filetype = match page.page.spec.style {
        PageStyle::Markdown => {
            lines.extend(source::markdown(body, &render));
            "markdown"
        }
        PageStyle::Colorscheme => {
            lines.extend(colorscheme(body, &render, themes, theme));
            "colors"
        }
    };
    if let Some(footer) = kind.and_then(|kind| kind.footer.as_deref()) {
        lines.extend(source::markdown(footer, &render));
    }
    let full_title = if page.page.spec.standalone_title {
        title.clone()
    } else {
        model.under_site(&title)
    };
    let description = page
        .page
        .description()
        .map(|description| model.expand(description, Some(&page.page)))
        .or_else(|| summary(kind, page).map(str::to_owned))
        .unwrap_or_default();
    let json_ld = match (kind, page.page.spec.json_ld) {
        (Some(kind), _) => kind
            .json_ld
            .as_ref()
            .map(|spec| typed_ld(site, kind, spec, page, &title)),
        (None, Some(PageJsonLd::Person)) => Some(person_ld(site, &model.profile.spec)),
        (None, None) => None,
    };
    let path = page.page.route().unwrap_or_default();
    let file = page.page.buffer();
    layout(
        Frame {
            site,
            model,
            theme,
            path: &path,
            file: &file,
            filetype,
            title: &full_title,
            description: &description,
            json_ld,
        },
        lines,
    )
}

/// The page's metadata as a YAML frontmatter: title, description, then its
/// type's fields in order. Lists always show; links show when one is set;
/// other fields show when set.
fn frontmatter(model: &Model, page: &Page, kind: Option<&PageKindSpec>) -> Vec<Markup> {
    let mut lines = vec![html! { p.mk { "---" } }];
    let title = model.expand(page.page.title(), Some(&page.page));
    lines.push(source::yaml_pair(0, "title", source::yaml_str(&title)));
    if let Some(description) = page.page.description() {
        let description = model.expand(description, Some(&page.page));
        lines.push(source::yaml_pair(
            0,
            "description",
            source::yaml_str(&description),
        ));
    }
    for (name, field) in kind.map(|kind| &kind.fields).into_iter().flatten() {
        let value = page.page.metadata.get(name);
        match (field.kind, value) {
            (FieldType::String | FieldType::Enum, Some(Value::String(text))) => {
                lines.push(source::yaml_pair(0, name, source::yaml_str(text)));
            }
            (FieldType::Int | FieldType::Bool, Some(value)) => {
                lines.push(source::yaml_pair(0, name, html! { span.n { (value) } }));
            }
            (FieldType::Url, Some(Value::String(url))) => {
                let shown = match safe_url(url) {
                    Some(url) => html! { a href=(url) { (url) } },
                    None => source::yaml_str(url),
                };
                lines.push(source::yaml_pair(0, name, shown));
            }
            (FieldType::List, value) => {
                let items: Vec<&str> = value
                    .and_then(Value::as_array)
                    .map(|items| items.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                lines.push(source::yaml_pair(
                    0,
                    name,
                    html! {
                        span.mk { "[" }
                        @for (i, item) in items.iter().enumerate() {
                            @if i > 0 { span.mk { ", " } }
                            span.s { (item) }
                        }
                        span.mk { "]" }
                    },
                ));
            }
            (FieldType::Links, Some(Value::Object(links))) if !links.is_empty() => {
                lines.push(source::yaml_key(0, name));
                for key in &field.keys {
                    if let Some(url) = links.get(key).and_then(Value::as_str).and_then(safe_url) {
                        lines.push(source::yaml_pair(2, key, html! { a href=(url) { (url) } }));
                    }
                }
            }
            _ => {}
        }
    }
    lines.push(html! { p.mk { "---" } });
    lines
}

/// A closed fold, as vim draws one, that opens without JavaScript.
fn fold(lines: Vec<Markup>, title: &str) -> Markup {
    html! {
        details.fold {
            summary.mk { (format!("+--{:>3} lines: title: {title}", lines.len())) }
            @for line in &lines { (line) }
        }
    }
}

/// `:colorscheme` as a page. Each line is a link that sets the cookie; its
/// palette rides along as custom properties for the live preview. The
/// page's markdown, if any, sits between the header and the list.
fn colorscheme(body: &str, render: &Render<'_>, themes: &[Theme], current: &Theme) -> Vec<Markup> {
    let mut lines = vec![
        html! { p.c { "\" :colorscheme: " (themes.len()) " palettes, * is yours" } },
        html! { p.c { "\" j k previews · Enter or click keeps it · leaving without Enter reverts" } },
        html! { p {} },
    ];
    if !body.is_empty() {
        lines.extend(source::markdown(body, render));
        lines.push(html! { p {} });
    }
    for candidate in themes {
        let mark = if candidate.slug == current.slug {
            "* "
        } else {
            "  "
        };
        lines.push(html! {
            p data-theme=(candidate.slug) style=(theme::vars(candidate)) {
                span.mk { (mark) }
                span.sw aria-hidden="true" { i {} i {} i {} i {} i {} }
                "  "
                a href=(format!("/theme/{}", candidate.slug)) { (candidate.title) }
                (pad(&candidate.title, 18))
                span.mk {
                    (candidate.slug) (pad(&candidate.slug, 18))
                    @if candidate.dark { "dark" } @else { "light" }
                }
            }
        });
    }
    lines
}

/// A collection's listing, in the manner of netrw.
pub fn listing(site: &Site, model: &Model, theme: &Theme, collection: &Collection) -> Markup {
    let name = &collection.name;
    let members = model.members(collection);
    let order = collection.spec.order.as_deref().map_or_else(
        || ", newest first".to_owned(),
        |order| format!(", ordered by {order}"),
    );
    let mut lines = vec![html! { p.c { "\" " (name) "/: " (members.len()) " indexed" (order) } }];
    let description = collection.spec.description.as_deref().unwrap_or_default();
    if !description.is_empty() {
        lines.push(html! { p.c { "\" " (description) } });
    }
    lines.push(html! { p {} });
    match model.kind_of(collection) {
        Some(kind) if !members.is_empty() => {
            let width = name_width(&members);
            lines.push(html! {
                p.k {
                    (pad_to("#", 3)) (pad_to("name", width))
                    @for column in &collection.spec.columns {
                        @let header = column.header.as_deref().unwrap_or(&column.field);
                        @match column.width {
                            Some(width) => (pad_to(header, width)),
                            None => (header),
                        }
                    }
                }
            });
            lines.extend(member_rows(collection, kind, &members));
        }
        kind => {
            let plural = kind.map_or("pages", |kind| kind.names.plural.as_str());
            lines.push(html! { p.c { "\" no " (plural) " yet." } });
        }
    }
    let route = format!("/{name}");
    let file = format!("{name}/");
    layout(
        Frame {
            site,
            model,
            theme,
            path: &route,
            file: &file,
            filetype: "netrw",
            title: &model.under_site(name),
            description,
            json_ld: None,
        },
        lines,
    )
}

/// `{{ collection }}`: the listing's rows, filtered and cut, and how many
/// more there are.
fn collection_rows(
    model: &Model,
    name: &str,
    filter: Option<&str>,
    limit: Option<usize>,
) -> Vec<Markup> {
    let Some(collection) = model.collection(name).filter(|c| c.spec.enabled) else {
        return Vec::new();
    };
    let Some(kind) = model.kind_of(collection) else {
        return Vec::new();
    };
    let members = model.members(collection);
    let filtered: Vec<&Page> = match filter {
        Some(field) => {
            let set: Vec<&Page> = members
                .iter()
                .copied()
                .filter(|member| member.page.metadata.contains_key(field))
                .collect();
            if set.is_empty() { members.clone() } else { set }
        }
        None => members.clone(),
    };
    let shown = &filtered[..limit.map_or(filtered.len(), |n| n.min(filtered.len()))];
    let mut lines = member_rows(collection, kind, shown);
    if members.len() > shown.len() {
        let more = members.len() - shown.len();
        lines.push(html! {
            p.c { "   … " (more) " more in " a href=(format!("/{name}")) { (name) "/" } }
        });
    }
    lines
}

fn member_rows(collection: &Collection, kind: &PageKindSpec, members: &[&Page]) -> Vec<Markup> {
    let width = name_width(members);
    let mut lines = Vec::with_capacity(members.len() * 2);
    for (i, member) in members.iter().enumerate() {
        let name = &member.page.name;
        lines.push(html! {
            p {
                span.n { (pad_to(&(i + 1).to_string(), 3)) }
                a href=(member.page.route().unwrap_or_default()) { (name) }
                (pad(name, width))
                @for column in &collection.spec.columns { (cell(column, kind, member)) }
            }
        });
        if let Some(summary) = summary(Some(kind), member) {
            lines.push(html! { p.dim { "   " (summary) } });
        }
    }
    lines
}

/// A listing cell: an enum in the ok color when good, anything else dim.
fn cell(column: &ListingColumn, kind: &PageKindSpec, member: &Page) -> Markup {
    let field = kind.fields.get(&column.field);
    let value = member.page.metadata.get(&column.field);
    let text = match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    };
    let text = match column.width {
        Some(width) => pad_to(&text, width),
        None => text,
    };
    match field {
        Some(field) if field.kind == FieldType::Enum => {
            let ok = value
                .and_then(Value::as_str)
                .is_some_and(|value| field.ok.iter().any(|ok| ok == value));
            html! { span class=(if ok { "ok" } else { "mk" }) { (text) } }
        }
        _ => html! { span.dim { (text) } },
    }
}

/// The value of a page type's summary field.
fn summary<'a>(kind: Option<&PageKindSpec>, page: &'a Page) -> Option<&'a str> {
    let field = kind?.summary.as_deref()?;
    page.page.metadata.get(field)?.as_str()
}

/// `{{ links }}`: the profile's links, one row each; `source` adds `src`.
fn links(profile: &ProfileSpec, with_source: bool) -> Vec<Markup> {
    let mut rows: Vec<(&str, String, &str)> = Vec::new();
    for (key, url) in [
        ("github", &profile.links.github),
        ("linkedin", &profile.links.linkedin),
    ] {
        if let Some(url) = url.as_deref().and_then(safe_url) {
            rows.push((key, url.to_owned(), display_url(url)));
        }
    }
    if let Some(email) = &profile.email {
        rows.push(("mail", format!("mailto:{email}"), email));
    }
    if with_source && let Some(url) = profile.links.source.as_deref().and_then(safe_url) {
        rows.push(("src", url.to_owned(), display_url(url)));
    }
    rows.into_iter()
        .map(|(key, href, label)| {
            html! { p { span.mk { "- " } (pad_to(key, 9)) a href=(href) rel="me" { (label) } } }
        })
        .collect()
}

pub fn help(site: &Site, model: &Model, theme: &Theme) -> Markup {
    let section = |title: &str, tag: &str| {
        vec![
            html! { p.mk { ("=".repeat(78)) } },
            html! { p { (title) (pad(title, 78 - tag.len() - 2)) span.tag id=(tag) { "*" (tag) "*" } } },
            html! { p {} },
        ]
    };
    let key =
        |keys: &str, text: Markup| html! { p { "  " span.n { (keys) } (pad(keys, 16)) (text) } };
    let lines: Vec<Markup> = [vec![
        html! { p { span.tag { "*hvpaiva.txt*" } "   For hldr " (hldr_core::VERSION) } },
        html! { p {} },
        html! { h1.h1 { "                    " (model.title().to_uppercase()) " · THE MANUAL" } },
        html! { p {} },
        html! { p { "You do not need any of this. Everything here is a link: the tree on the left, the tabs on top, the words in the files. Click. The keys below are for people who already speak vim." } },
        html! { p {} },
        html! { p { a href="#hldr-move" { "|hldr-move|" } "       moving around" } },
        html! { p { a href="#hldr-files" { "|hldr-files|" } "      files, buffers, the tree" } },
        html! { p { a href="#hldr-search" { "|hldr-search|" } "     searching" } },
        html! { p { a href="#hldr-commands" { "|hldr-commands|" } "   : commands" } },
        html! { p {} },
    ],
    section("MOVING AROUND", "hldr-move"),
    vec![
        key("j  k", html! { "line down / up, with a count: " span.n { "5j" } }),
        key("gg  G", html! { "first / last line; " span.n { "12G" } " goes to line 12" }),
        key("CTRL-D  CTRL-U", html! { "half a page down / up" }),
        key("{  }", html! { "previous / next paragraph" }),
        key("zz", html! { "center the cursor line" }),
        key("Enter  CTRL-]", html! { "follow the link on the cursor line" }),
        key("gx", html! { "open it in a new tab" }),
        key("Tab", html! { "next link, like this help" }),
        key("CTRL-O  CTRL-I", html! { "jump back / forward" }),
        html! { p {} },
    ],
    section("FILES", "hldr-files"),
    vec![
        key("space e", html! { "toggle the tree        " span.c { "also :Neotree  :NvimTreeToggle" } }),
        key("CTRL-W h  l", html! { "focus the tree / the file" }),
        html! { p { (pad("", 18)) "in the tree: " span.n { "j k" } " move, " span.n { "l" } " or " span.n { "Enter" } " open, " span.n { "h" } " fold" } },
        key("space ff", html! { "find a file by name    " span.c { "also :find  :Telescope" } }),
        key("gt  gT", html! { "next / previous buffer " span.c { "also :bn  :bp" } }),
        key(":bd", html! { "close this buffer" }),
        key(":ls", html! { "list buffers" }),
        html! { p {} },
    ],
    section("SEARCHING", "hldr-search"),
    vec![
        key("/word", html! { "search forward in this file" }),
        key("?word", html! { "search backward" }),
        key("n  N", html! { "next / previous match" }),
        key(":noh", html! { "clear the highlight" }),
        html! { p { (pad("", 18)) "lowercase ignores case; one capital makes it exact" } },
        html! { p {} },
    ],
    section("COMMANDS", "hldr-commands"),
    vec![
        key(":e {file}", html! { "open a file, Tab completes" }),
        key(":colorscheme", html! { "the palette page; " span.n { ":colo {name}" } " sets one" }),
        key(":set nu rnu wrap", html! { "and their " span.n { "no" } "… forms; " span.n { ":set ch=1" } " keeps this line" }),
        key(":checkhealth", html! { "is the site up?" }),
        key(":help {topic}", html! { "jump here; " span.n { ":q" } " to… well, try it" }),
        html! { p {} },
        html! { p.c { "Not everything is documented." } },
    ]]
    .concat();
    layout(
        Frame {
            site,
            model,
            theme,
            path: "/help",
            file: "help.txt",
            filetype: "help",
            title: &model.under_site("help"),
            description: &format!(
                "Keys and commands for {}. None of them are required.",
                model.title()
            ),
            json_ld: None,
        },
        lines,
    )
}

/// What `/health` reports beyond the build: where content comes from and how
/// its last sync went.
pub struct Health<'a> {
    /// Such as `github.com/hvpaiva/hldr-content@main`.
    pub source: String,
    /// `owner/name` when content comes from GitHub, for links to commits.
    pub repository: Option<&'a str>,
    pub sync: &'a SyncState,
    /// Pages of each enabled collection, drafts included, with the page
    /// type's singular and plural.
    pub counts: Vec<(usize, &'a str, &'a str)>,
    pub themes: usize,
}

/// `:checkhealth` as a page: rendered on the server from the state the
/// private API reports, so it needs no script and shows what a visitor gets.
pub fn health(site: &Site, model: &Model, report: &Health<'_>, theme: &Theme) -> Markup {
    let ok = |text: Markup| html! { p { span.ok { "- OK" } " " (text) } };
    let warn = |text: Markup| html! { p { span.warn { "- WARNING" } " " (text) } };
    let error = |text: Markup| html! { p { span.err { "- ERROR" } " " (text) } };
    let commit = |base: &str, sha: &str| {
        let short = sha.get(..12).unwrap_or(sha);
        html! { a href=(format!("{base}/commit/{sha}")) { (short) } }
    };
    let built = if hldr_core::REVISION.len() == 40 {
        html! { ", built from " (commit(REPOSITORY, hldr_core::REVISION)) }
    } else {
        html! { ", built from " span.mk { (hldr_core::REVISION) } }
    };
    let version = if hldr_core::VERSION == "dev" {
        html! { "hldr dev" }
    } else {
        let tag = format!("v{}", hldr_core::VERSION);
        html! { a href=(format!("{REPOSITORY}/releases/tag/{tag}")) { "hldr " (hldr_core::VERSION) } }
    };
    let sync = report.sync;
    let served = match (&sync.synced_at, &sync.revision, report.repository) {
        (None, _, _) => error(html! { "nothing synced yet: the site has no content to serve" }),
        (Some(at), Some(revision), Some(repo)) => ok(html! {
            "serving " (commit(&format!("https://github.com/{repo}"), revision)) ", synced " (at)
        }),
        (Some(at), _, _) => ok(html! { "serving the directory as it is on disk, synced " (at) }),
    };
    let attempt = sync.last_attempt_at.as_deref().unwrap_or("never");
    let mut indexed: Vec<String> = report
        .counts
        .iter()
        .map(|&(n, singular, plural)| format!("{n} {}", if n == 1 { singular } else { plural }))
        .collect();
    indexed.push(count(report.themes, "theme"));
    let picker = model
        .picker()
        .and_then(|page| page.page.route())
        .unwrap_or_else(|| "/theme".to_owned());
    let mut lines = vec![
        html! { h1.h1 { "hldr: require(\"hldr.health\").check()" } },
        html! { p {} },
        html! { h2.h2 { span.mk { "## " } "server" } },
        ok(html! { (version) (built) }),
        ok(html! { "database answers; " (sentence(&indexed)) " indexed" }),
        html! { p {} },
        html! { h2.h2 { span.mk { "## " } "content" } },
        ok(html! { "source " (report.source) }),
        served,
    ];
    match &sync.last_error {
        None => lines.push(ok(html! { "last sync attempt " (attempt) " succeeded" })),
        Some(message) => {
            lines.push(warn(html! {
                "last sync attempt " (attempt) " failed; the revision above is still served:"
            }));
            for line in message.lines() {
                lines.push(html! { p.err { "    " (line) } });
            }
        }
    }
    lines.extend([
        html! { p {} },
        html! { h2.h2 { span.mk { "## " } "ui" } },
        ok(html! { "colorscheme " a href=(picker) { (theme.slug) } }),
        ok(html! { "javascript: optional; every page is a URL" }),
    ]);
    layout(
        Frame {
            site,
            model,
            theme,
            path: "/health",
            file: "[checkhealth]",
            filetype: "checkhealth",
            title: &model.under_site("checkhealth"),
            description: "Server version, content revision and sync state.",
            json_ld: None,
        },
        lines,
    )
}

/// `a`, `a and b`, `a, b and c`.
fn sentence(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Every 404: the lines vim prints for a missing file, then the not-found
/// page's markdown.
pub fn not_found(
    site: &Site,
    model: &Model,
    theme: &Theme,
    path: &str,
    detail: &str,
    content: Option<&str>,
) -> Markup {
    let file = path.trim_start_matches('/');
    let mut lines = vec![
        html! { p.err { "E447: Can't find file \"" (file) "\" in path" } },
        html! { p.c { "\" " (detail) } },
        html! { p {} },
    ];
    let page = model.not_found();
    if let Some(content) = content {
        let render = Render::new(model, page);
        let body = content.trim_start_matches(['\r', '\n']).trim_end();
        lines.extend(source::markdown(body, &render));
    }
    let title = page.map_or_else(
        || "404".to_owned(),
        |page| model.expand(page.page.title(), Some(&page.page)),
    );
    layout(
        Frame {
            site,
            model,
            theme,
            path,
            file: if file.is_empty() { "[No Name]" } else { file },
            filetype: "",
            title: &model.under_site(&title),
            description: detail,
            json_ld: None,
        },
        lines,
    )
}

fn count(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

fn last_segment(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn name_width(members: &[&Page]) -> usize {
    members
        .iter()
        .map(|member| member.page.name.chars().count())
        .max()
        .unwrap_or(0)
        .max(10)
        + 2
}

fn pad(text: &str, width: usize) -> String {
    " ".repeat(width.saturating_sub(text.chars().count()).max(1))
}

fn pad_to(text: &str, width: usize) -> String {
    format!("{text}{}", pad(text, width))
}

pub fn display_url(url: &str) -> &str {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
}

fn oneline(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// JSON-LD with its keys sorted, whatever order serde_json keeps maps in,
/// and nothing that closes the script element.
fn ld_json(entries: BTreeMap<&str, Value>) -> String {
    serde_json::to_string(&entries)
        .unwrap_or_default()
        .replace('<', LESS_THAN)
}

/// `<` as a JSON escape: JSON-LD holding it can never close its script.
const LESS_THAN: &str = concat!("\\", "u003c");

fn person_ld(site: &Site, profile: &ProfileSpec) -> String {
    let same_as: Vec<&String> = [&profile.links.github, &profile.links.linkedin]
        .into_iter()
        .flatten()
        .collect();
    ld_json(BTreeMap::from([
        ("@context", Value::from("https://schema.org")),
        ("@type", Value::from("Person")),
        ("name", Value::from(profile.name.as_str())),
        ("jobTitle", Value::from(profile.headline.as_str())),
        ("url", Value::from(site.origin.as_str())),
        (
            "email",
            profile.email.as_deref().map_or(Value::Null, Value::from),
        ),
        (
            "sameAs",
            Value::from(same_as.into_iter().cloned().collect::<Vec<_>>()),
        ),
    ]))
}

/// A page type's JSON-LD: `name` and `url` from the page, the mapped keys
/// from its fields, and the summary as the description unless mapped.
fn typed_ld(
    site: &Site,
    kind: &PageKindSpec,
    spec: &hldr_core::spec::JsonLdSpec,
    page: &Page,
    title: &str,
) -> String {
    let url = format!("{}{}", site.origin, page.page.route().unwrap_or_default());
    let mut entries = BTreeMap::from([
        ("@context", Value::from("https://schema.org")),
        ("@type", Value::from(spec.kind.as_str())),
        ("name", Value::from(title)),
        ("url", Value::from(url)),
    ]);
    let field = |source: &str| -> Value {
        match source {
            "title" => Value::from(title),
            "description" => page.page.description().map_or(Value::Null, Value::from),
            _ => {
                let mut keys = source.split('.');
                let first = keys.next().and_then(|key| page.page.metadata.get(key));
                keys.fold(first, |node, key| node.and_then(|node| node.get(key)))
                    .cloned()
                    .unwrap_or(Value::Null)
            }
        }
    };
    if let Some(summary) = &kind.summary {
        entries.insert("description", field(summary));
    }
    for (key, source) in &spec.map {
        entries.insert(key, field(source));
    }
    ld_json(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ld_json_neutralizes_script_breakout() {
        let out = ld_json(BTreeMap::from([(
            "name",
            Value::from("</script><img src=x onerror=alert(1)>"),
        )]));
        assert!(!out.contains("</script>"));
        assert!(out.contains(&format!("{LESS_THAN}/script>")));
    }

    #[test]
    fn ld_json_sorts_its_keys() {
        let out = ld_json(BTreeMap::from([
            ("b", Value::from(1)),
            ("@a", Value::from(2)),
        ]));
        assert_eq!(out, r#"{"@a":2,"b":1}"#);
    }

    #[test]
    fn pad_never_collapses_columns() {
        assert_eq!(pad("longer-than-width", 4), " ");
        assert_eq!(pad_to("#", 3), "#  ");
    }

    #[test]
    fn sentences_join_like_prose() {
        let words = |list: &[&str]| list.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        assert_eq!(sentence(&words(&["1 theme"])), "1 theme");
        assert_eq!(
            sentence(&words(&["1 project", "2 themes"])),
            "1 project and 2 themes"
        );
        assert_eq!(sentence(&words(&["a", "b", "c"])), "a, b and c");
    }

    #[test]
    fn folds_count_their_lines() {
        let fold = fold(vec![html! { p { "a" } }, html! { p { "b" } }], "hldr").0;
        assert!(
            fold.starts_with(
                r#"<details class="fold"><summary class="mk">+--  2 lines: title: hldr</summary>"#
            ),
            "{fold}"
        );
    }
}
