//! The site as an editor: a file tree, one buffer per route, a statusline.
//! Every page is complete without JavaScript; `keys.js` and `vim.js` only
//! add the keyboard layer on top.

use hldr_core::{Profile, Project, ProjectSummary};
use maud::{DOCTYPE, Markup, PreEscaped, html};

use crate::Site;
use crate::source;
use crate::theme::{THEMES, Theme};

const ASCII_BANNER: &str = "\
ooooo ooooo ooooo
 888   888   888
 888   888   888
 888ooo888   888
 888   888   888     o
o888o o888o o888oooo88

ooooooooo  oooooooooo
 888    88o 888    888
 888    888 888oooo88
 888    888 888  88o
o888ooo88  o888o  88o8";

const WRAP: usize = 72;
const TREE_OPEN_MAX: usize = 8;
/// Commands that only exist once vim.js can run them.
const JS_ONLY: [&str; 2] = ["find", "checkhealth"];

/// What every page needs to draw the file tree.
pub struct Tree<'a> {
    pub projects: &'a [ProjectSummary],
    pub blog_enabled: bool,
}

struct Page<'a> {
    site: &'a Site,
    tree: &'a Tree<'a>,
    theme: &'static Theme,
    path: &'a str,
    file: &'a str,
    filetype: &'a str,
    title: &'a str,
    description: &'a str,
    json_ld: Option<String>,
}

fn layout(page: Page<'_>, lines: Vec<Markup>) -> Markup {
    let canonical = format!("{}{}", page.site.origin, page.path);
    let description = oneline(page.description);
    let count = page.tree.projects.len();
    let tab = page
        .file
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(page.file);
    let tab = if page.file.ends_with('/') {
        format!("{tab}/")
    } else {
        tab.to_owned()
    };
    html! {
        (DOCTYPE)
        html lang="en" data-theme=(page.theme.slug) {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover";
                title { (page.title) }
                meta name="description" content=(description);
                link rel="canonical" href=(canonical);
                meta property="og:title" content=(page.title);
                meta property="og:description" content=(description);
                meta property="og:url" content=(canonical);
                meta property="og:type" content="website";
                meta name="twitter:card" content="summary";
                link rel="icon" href=(format!("/favicon.svg?t={}", page.theme.slug)) type="image/svg+xml";
                link rel="stylesheet" href=(format!("/style.css?v={}", hldr_core::VERSION));
                style { (PreEscaped(page.theme.root_css())) }
                @if let Some(json_ld) = &page.json_ld {
                    script type="application/ld+json" { (PreEscaped(json_ld)) }
                }
            }
            body data-path=(page.path) data-file=(page.file) data-ft=(page.filetype) {
                a.skip-link href="#win" { "skip to content" }
                input #tt .tt type="checkbox" aria-label="toggle the file tree";
                div #ed .ed {
                    nav #tree .tree aria-label="files" { (tree(page.tree, page.path)) }
                    div.tabs {
                        label.tgl for="tt" title="toggle the tree (space e)" { "≡" }
                        div #bufs .bufs {
                            span.tab.on { a href=(page.path) aria-current="page" { (tab) } }
                        }
                    }
                    main #win .win tabindex="-1" {
                        article.buf data-path=(page.file) data-ft=(page.filetype) {
                            @for line in &lines { (line) }
                        }
                    }
                    footer #status .status {
                        span #mode .mode { "NORMAL" }
                        span.file { label for="tt" { (page.file) } }
                        span.ro { "[RO]" }
                        span.fill {}
                        span #showcmd .showcmd aria-hidden="true" {}
                        span.chip { a href="/help" { ":help" } }
                        span.ft { (page.filetype) }
                        span.ver {
                            a href="/projects" {
                                (count) @if count == 1 { " project" } @else { " projects" }
                            }
                            " · hldr " (hldr_core::VERSION)
                        }
                        span #pos .pos { (lines.len()) "L" }
                    }
                }
                script src=(format!("/keys.js?v={}", hldr_core::VERSION)) defer {}
            }
        }
    }
}

fn tree(tree: &Tree<'_>, current: &str) -> Markup {
    let in_projects = current.starts_with("/projects");
    let open_projects = in_projects || tree.projects.len() <= TREE_OPEN_MAX;
    html! {
        details.sec data-fold="files" open {
            summary.hd { "hvpaiva.dev" }
            (tree_file("/", "README.md", current))
            (tree_file("/about", "about.md", current))
            (tree_file("/profile", "profile.yaml", current))
            details.dir data-fold="projects" open[open_projects] {
                summary.row.dir class=[(current == "/projects").then_some("cur")] {
                    a href="/projects" { "projects/" }
                    span.badge.off { (tree.projects.len()) }
                }
                div.kids {
                    @for project in tree.projects {
                        @let href = format!("/projects/{}", project.slug);
                        div.row class=[(current == href).then_some("cur")] {
                            span.ic { "·" }
                            a href=(href) data-file=(format!("projects/{}.md", project.slug)) { (project.slug) ".md" }
                            @if project.status.as_str() == "active" { span.badge { "active" } }
                        }
                    }
                }
            }
            @if tree.blog_enabled {
                (tree_file("/blog", "blog/", current))
            } @else {
                div.row.dir.off { span.ic { "▸" } span.grow { "blog/" } span.badge.off { "off" } }
            }
        }
        details.sec data-fold="commands" open {
            summary.hd { "commands" }
            (tree_cmd("/theme", ":", "colorscheme", None, "colorscheme", current))
            (tree_cmd("/projects", "/", "find file", Some("␣ff"), "find", current))
            (tree_cmd("/help", ":", "help", Some("F1"), "help", current))
            (tree_cmd("/healthz", ":", "checkhealth", None, "checkhealth", current))
        }
        details.sec data-fold="elsewhere" open {
            summary.hd { "elsewhere" }
            div.row { span.ic { "↗" } a href="https://github.com/hvpaiva" rel="me" { "github" } }
            div.row { span.ic { "↗" } a href="https://linkedin.com/in/hvpaiva" rel="me" { "linkedin" } }
            div.row { span.ic { "@" } a href="mailto:contact@hvpaiva.dev" { "mail" } }
        }
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

pub fn home(
    site: &Site,
    tree: &Tree<'_>,
    profile: &Profile,
    highlighted: &[ProjectSummary],
    theme: &'static Theme,
) -> Markup {
    let mut lines = vec![
        html! { pre.ban role="img" aria-label="HLDR" { (ASCII_BANNER) } },
        html! { p {} },
        html! { h1.h1 { span.mk { "# " } (profile.name) } },
        html! { p.q { span.mk { "> " } (profile.headline) } },
        html! { p {} },
        html! { p { (profile.bio.trim()) } },
        html! { p {} },
        html! { h2.h2 { span.mk { "## " } a href="/projects" { "Projects" } } },
    ];
    let shown = highlighted.get(..3).unwrap_or(highlighted);
    lines.extend(project_rows(shown));
    if tree.projects.len() > shown.len() {
        let more = tree.projects.len() - shown.len();
        lines.push(html! { p.c { "   … " (more) " more in " a href="/projects" { "projects/" } } });
    }
    lines.push(html! { p {} });
    lines.push(html! { h2.h2 { span.mk { "## " } "About" } });
    if let Some(first) = profile
        .about_source
        .lines()
        .find(|line| !line.trim().is_empty())
    {
        lines.push(
            html! { p { (source::inline(first)) " More in " a href="/about" { "about.md" } "." } },
        );
    }
    lines.push(html! { p {} });
    lines.push(html! { h2.h2 { span.mk { "## " } "Elsewhere" } });
    lines.extend(elsewhere(profile, true));
    lines.push(html! { p {} });
    lines.push(html! { h2.h2 { span.mk { "## " } "Also in a terminal" } });
    lines.push(html! { p.c { "The same content over curl, plain text in 80 columns." } });
    lines.push(html! { p.mk { "```sh" } });
    for (path, label, comment) in [
        ("/", "", "# README.md"),
        ("/about", "/about", "# about.md"),
        ("/projects", "/projects", "# projects/"),
    ] {
        let target = format!("{}{label}", site.host);
        lines.push(html! {
            p { span.n { "curl" } " " a href=(path) { (target) } (pad(&target, 26)) span.c { (comment) } }
        });
    }
    lines.push(html! { p.mk { "```" } });
    layout(
        Page {
            site,
            tree,
            theme,
            path: "/",
            file: "README.md",
            filetype: "markdown",
            title: &profile.name,
            description: &profile.bio,
            json_ld: Some(person_ld(site, profile)),
        },
        lines,
    )
}

pub fn about(site: &Site, tree: &Tree<'_>, profile: &Profile, theme: &'static Theme) -> Markup {
    let mut lines = vec![html! { h1.h1 { span.mk { "# " } "about" } }, html! { p {} }];
    lines.extend(source::markdown(profile.about_source.trim_end()));
    lines.push(html! { p {} });
    lines.push(html! { h2.h2 { span.mk { "## " } "Contact" } });
    lines.extend(elsewhere(profile, false));
    layout(
        Page {
            site,
            tree,
            theme,
            path: "/about",
            file: "about.md",
            filetype: "markdown",
            title: "about · hvpaiva.dev",
            description: &profile.headline,
            json_ld: Some(person_ld(site, profile)),
        },
        lines,
    )
}

pub fn profile(site: &Site, tree: &Tree<'_>, profile: &Profile, theme: &'static Theme) -> Markup {
    let mut lines = vec![
        source::yaml_pair(0, "name", source::yaml_str(&profile.name)),
        source::yaml_pair(0, "headline", source::yaml_str(&profile.headline)),
        source::yaml_pair(0, "bio", html! { span.mk { ">" } }),
    ];
    for line in source::wrap(profile.bio.trim(), WRAP) {
        lines.push(html! { p { "  " span.s { (line) } } });
    }
    if let Some(email) = &profile.email {
        lines.push(source::yaml_pair(
            0,
            "email",
            html! { a href=(format!("mailto:{email}")) { (email) } },
        ));
    }
    lines.push(source::yaml_key(0, "links"));
    for (key, value) in [
        ("github", &profile.links.github),
        ("linkedin", &profile.links.linkedin),
        ("source", &profile.links.source),
    ] {
        if let Some(url) = value {
            lines.push(source::yaml_pair(
                2,
                key,
                html! { a href=(url) rel="me" { (url) } },
            ));
        }
    }
    lines.push(source::yaml_pair(
        0,
        "about",
        html! { span.mk { "|" } " " span.c { "# shown in " } a href="/about" { "about.md" } },
    ));
    layout(
        Page {
            site,
            tree,
            theme,
            path: "/profile",
            file: "profile.yaml",
            filetype: "yaml",
            title: "profile · hvpaiva.dev",
            description: &profile.headline,
            json_ld: None,
        },
        lines,
    )
}

pub fn projects_index(
    site: &Site,
    tree: &Tree<'_>,
    description: &str,
    theme: &'static Theme,
) -> Markup {
    let count = tree.projects.len();
    let mut lines = vec![
        html! { p.c { "\" projects/: " (count) " indexed, ordered by highlight" } },
        html! { p.c { "\" " (description) } },
        html! { p {} },
    ];
    if tree.projects.is_empty() {
        lines.push(html! { p.c { "\" no projects yet." } });
    } else {
        let width = name_width(tree.projects);
        lines.push(html! {
            p.k { (pad_to("#", 3)) (pad_to("name", width)) (pad_to("status", 9)) "stack" }
        });
        lines.extend(project_rows(tree.projects));
    }
    layout(
        Page {
            site,
            tree,
            theme,
            path: "/projects",
            file: "projects/",
            filetype: "netrw",
            title: "projects · hvpaiva.dev",
            description,
            json_ld: None,
        },
        lines,
    )
}

pub fn project_page(
    site: &Site,
    tree: &Tree<'_>,
    project: &Project,
    theme: &'static Theme,
) -> Markup {
    let path = format!("/projects/{}", project.slug);
    let file = format!("projects/{}.md", project.slug);
    let title = format!("{} · hvpaiva.dev", project.title);
    let mut lines = vec![
        html! { p.mk { "---" } },
        source::yaml_pair(0, "title", source::yaml_str(&project.title)),
        source::yaml_pair(0, "tagline", source::yaml_str(&project.tagline)),
        source::yaml_pair(0, "status", source::yaml_str(project.status.as_str())),
    ];
    if let Some(highlight) = project.highlight {
        lines.push(source::yaml_pair(
            0,
            "highlight",
            html! { span.n { (highlight) } },
        ));
    }
    lines.push(source::yaml_pair(
        0,
        "tags",
        html! {
            span.mk { "[" }
            @for (i, tag) in project.tags.iter().enumerate() {
                @if i > 0 { span.mk { ", " } }
                span.s { (tag) }
            }
            span.mk { "]" }
        },
    ));
    let links = [
        ("repo", &project.links.repo),
        ("docs", &project.links.docs),
        ("demo", &project.links.demo),
    ];
    if links.iter().any(|(_, url)| url.is_some()) {
        lines.push(source::yaml_key(0, "links"));
        for (key, url) in links {
            if let Some(url) = url.as_deref().and_then(source::safe_url) {
                lines.push(source::yaml_pair(2, key, html! { a href=(url) { (url) } }));
            }
        }
    }
    if let Some(repo) = &project.github_repo {
        lines.push(source::yaml_pair(0, "github", source::yaml_str(repo)));
    }
    lines.push(html! { p.mk { "---" } });
    lines.extend(source::markdown(project.body_source.trim_end()));
    lines.push(html! { p {} });
    if let Some(repo) = project.links.repo.as_deref().and_then(source::safe_url) {
        lines
            .push(html! { p { span.n { "git clone" } " " a href=(repo) { (display_url(repo)) } } });
    }
    lines.push(html! { p { a href="/projects" { "← projects/" } } });
    layout(
        Page {
            site,
            tree,
            theme,
            path: &path,
            file: &file,
            filetype: "markdown",
            title: &title,
            description: &project.tagline,
            json_ld: Some(software_ld(site, project)),
        },
        lines,
    )
}

/// `:colorscheme` as a page. Each line is a link that sets the cookie;
/// its palette rides along as custom properties for the live preview.
pub fn themes_index(site: &Site, tree: &Tree<'_>, theme: &'static Theme) -> Markup {
    let mut lines = vec![
        html! { p.c { "\" :colorscheme: " (THEMES.len()) " Omarchy palettes, * is yours" } },
        html! { p.c { "\" j k previews · Enter or click keeps it · leaving without Enter reverts" } },
        html! { p {} },
    ];
    for candidate in THEMES {
        let mark = if candidate.slug == theme.slug {
            "* "
        } else {
            "  "
        };
        lines.push(html! {
            p data-theme=(candidate.slug) style=(candidate.vars()) {
                span.mk { (mark) }
                span.sw aria-hidden="true" { i {} i {} i {} i {} i {} }
                "  "
                a href=(format!("/theme/{}", candidate.slug)) { (candidate.name) }
                (pad(candidate.name, 18))
                span.mk {
                    (candidate.slug) (pad(candidate.slug, 18))
                    @if candidate.dark { "dark" } @else { "light" }
                }
            }
        });
    }
    layout(
        Page {
            site,
            tree,
            theme,
            path: "/theme",
            file: "[colorscheme]",
            filetype: "colors",
            title: "theme · hvpaiva.dev",
            description: "Pick a palette. Omarchy themes, applied to this page.",
            json_ld: None,
        },
        lines,
    )
}

pub fn help(site: &Site, tree: &Tree<'_>, theme: &'static Theme) -> Markup {
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
        html! { p { span.tag { "*hvpaiva.txt*" } "   For hldr " (hldr_core::VERSION) "              Last change: 2026 Sep 21" } },
        html! { p {} },
        html! { h1.h1 { "                    HVPAIVA.DEV · THE MANUAL" } },
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
        Page {
            site,
            tree,
            theme,
            path: "/help",
            file: "help.txt",
            filetype: "help",
            title: "help · hvpaiva.dev",
            description: "Keys and commands for hvpaiva.dev. None of them are required.",
            json_ld: None,
        },
        lines,
    )
}

pub fn not_found(
    site: &Site,
    tree: &Tree<'_>,
    path: &str,
    detail: &str,
    theme: &'static Theme,
) -> Markup {
    let file = path.trim_start_matches('/');
    let lines = vec![
        html! { p.err { "E447: Can't find file \"" (file) "\" in path" } },
        html! { p.c { "\" " (detail) } },
        html! { p {} },
        html! { p { a href="/" { "README.md" } "      start here" } },
        html! { p { a href="/projects" { "projects/" } "      everything indexed" } },
        html! { p { a href="/help" { "help.txt" } "       keys and commands" } },
    ];
    layout(
        Page {
            site,
            tree,
            theme,
            path,
            file: if file.is_empty() { "[No Name]" } else { file },
            filetype: "",
            title: "404 · hvpaiva.dev",
            description: detail,
            json_ld: None,
        },
        lines,
    )
}

fn project_rows(projects: &[ProjectSummary]) -> Vec<Markup> {
    let width = name_width(projects);
    let mut lines = Vec::with_capacity(projects.len() * 2);
    for (i, project) in projects.iter().enumerate() {
        let status = project.status.as_str();
        lines.push(html! {
            p {
                span.n { (pad_to(&(i + 1).to_string(), 3)) }
                a href=(format!("/projects/{}", project.slug)) { (project.slug) }
                (pad(&project.slug, width))
                span class=(if status == "active" { "ok" } else { "mk" }) { (pad_to(status, 9)) }
                span.dim { (project.tags.join(", ")) }
            }
        });
        lines.push(html! { p.dim { "   " (project.tagline) } });
    }
    lines
}

fn elsewhere(profile: &Profile, with_source: bool) -> Vec<Markup> {
    let mut rows: Vec<(&str, String, &str)> = Vec::new();
    if let Some(url) = &profile.links.github {
        rows.push(("github", url.clone(), display_url(url)));
    }
    if let Some(url) = &profile.links.linkedin {
        rows.push(("linkedin", url.clone(), display_url(url)));
    }
    if let Some(email) = &profile.email {
        rows.push(("mail", format!("mailto:{email}"), email));
    }
    if with_source && let Some(url) = &profile.links.source {
        rows.push(("src", url.clone(), display_url(url)));
    }
    rows.into_iter()
        .map(|(key, href, label)| {
            html! { p { span.mk { "- " } (pad_to(key, 9)) a href=(href) rel="me" { (label) } } }
        })
        .collect()
}

fn name_width(projects: &[ProjectSummary]) -> usize {
    projects
        .iter()
        .map(|project| project.slug.chars().count())
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

fn ld_json(value: serde_json::Value) -> String {
    value.to_string().replace('<', r"\u003c")
}

fn person_ld(site: &Site, profile: &Profile) -> String {
    let mut same_as = Vec::new();
    if let Some(github) = &profile.links.github {
        same_as.push(github.clone());
    }
    if let Some(linkedin) = &profile.links.linkedin {
        same_as.push(linkedin.clone());
    }
    ld_json(serde_json::json!({
        "@context": "https://schema.org",
        "@type": "Person",
        "name": profile.name,
        "jobTitle": profile.headline,
        "url": site.origin,
        "email": profile.email,
        "sameAs": same_as,
    }))
}

fn software_ld(site: &Site, project: &Project) -> String {
    ld_json(serde_json::json!({
        "@context": "https://schema.org",
        "@type": "SoftwareSourceCode",
        "name": project.title,
        "description": project.tagline,
        "url": format!("{}/projects/{}", site.origin, project.slug),
        "codeRepository": project.links.repo,
        "programmingLanguage": project.tags,
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn ld_json_neutralizes_script_breakout() {
        let out = super::ld_json(serde_json::json!({
            "name": "</script><img src=x onerror=alert(1)>"
        }));
        assert!(!out.contains("</script>"));
        assert!(out.contains(r"\u003c/script>"));
    }

    #[test]
    fn pad_never_collapses_columns() {
        assert_eq!(super::pad("longer-than-width", 4), " ");
        assert_eq!(super::pad_to("#", 3), "#  ");
    }
}
