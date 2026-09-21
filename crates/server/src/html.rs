//! The site as an editor: a file tree, one buffer per route, a statusline.
//! Every page is complete without JavaScript; `keys.js` and `vim.js` only
//! add the keyboard layer on top.

use hldr_core::{Profile, Project, ProjectSummary, SiteConfig, SyncState, Theme};
use maud::{DOCTYPE, Markup, PreEscaped, html};

use crate::Site;
use crate::source;
use crate::theme;

const WRAP: usize = 72;
const TREE_OPEN_MAX: usize = 8;
/// Commands that only exist once vim.js can run them.
const JS_ONLY: [&str; 1] = ["find"];
/// Where the server's source lives, for links to its releases and commits.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// What every page needs around its buffer: the file tree and the site's
/// identity.
pub struct Tree<'a> {
    pub projects: &'a [ProjectSummary],
    pub site: &'a SiteConfig,
    pub profile: &'a Profile,
}

impl Tree<'_> {
    /// A page title under the site's name.
    fn title(&self, page: &str) -> String {
        format!("{page} · {}", self.site.title)
    }
}

struct Page<'a> {
    site: &'a Site,
    tree: &'a Tree<'a>,
    theme: &'a Theme,
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
                style { (PreEscaped(theme::root_css(page.theme))) }
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
                            " · "
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

fn tree(tree: &Tree<'_>, current: &str) -> Markup {
    let in_projects = current.starts_with("/projects");
    let open_projects = in_projects || tree.projects.len() <= TREE_OPEN_MAX;
    html! {
        details.sec data-fold="files" open {
            summary.hd { (tree.site.title) }
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
            @if tree.site.blog_enabled {
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
            (tree_cmd("/health", ":", "checkhealth", None, "checkhealth", current))
        }
        details.sec data-fold="elsewhere" open {
            summary.hd { "elsewhere" }
            @if let Some(url) = tree.profile.links.github.as_deref().and_then(source::safe_url) {
                div.row { span.ic { "↗" } a href=(url) rel="me" { "github" } }
            }
            @if let Some(url) = tree.profile.links.linkedin.as_deref().and_then(source::safe_url) {
                div.row { span.ic { "↗" } a href=(url) rel="me" { "linkedin" } }
            }
            @if let Some(email) = &tree.profile.email {
                div.row { span.ic { "@" } a href=(format!("mailto:{email}")) { "mail" } }
            }
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
    theme: &Theme,
) -> Markup {
    let mut lines = Vec::new();
    if let Some(banner) = &tree.site.banner {
        lines.push(html! { pre.ban role="img" aria-label=(banner.alt) { (banner.art) } });
        lines.push(html! { p {} });
    }
    lines.extend([
        html! { h1.h1 { span.mk { "# " } (profile.name) } },
        html! { p.q { span.mk { "> " } (profile.headline) } },
        html! { p {} },
        html! { p { (profile.bio.trim()) } },
        html! { p {} },
        html! { h2.h2 { span.mk { "## " } a href="/projects" { "Projects" } } },
    ]);
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

pub fn about(site: &Site, tree: &Tree<'_>, profile: &Profile, theme: &Theme) -> Markup {
    let mut lines = vec![html! { h1.h1 { span.mk { "# " } "about" } }, html! { p {} }];
    lines.extend(source::markdown(
        profile
            .about_source
            .trim_start_matches(['\r', '\n'])
            .trim_end(),
    ));
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
            title: &tree.title("about"),
            description: &profile.headline,
            json_ld: Some(person_ld(site, profile)),
        },
        lines,
    )
}

pub fn profile(site: &Site, tree: &Tree<'_>, profile: &Profile, theme: &Theme) -> Markup {
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
            title: &tree.title("profile"),
            description: &profile.headline,
            json_ld: None,
        },
        lines,
    )
}

pub fn projects_index(site: &Site, tree: &Tree<'_>, theme: &Theme) -> Markup {
    let description = tree.site.descriptions.projects.as_str();
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
            title: &tree.title("projects"),
            description,
            json_ld: None,
        },
        lines,
    )
}

pub fn project_page(site: &Site, tree: &Tree<'_>, project: &Project, theme: &Theme) -> Markup {
    let path = format!("/projects/{}", project.slug);
    let file = format!("projects/{}.md", project.slug);
    let title = tree.title(&project.title);
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
pub fn themes_index(site: &Site, tree: &Tree<'_>, themes: &[Theme], theme: &Theme) -> Markup {
    let mut lines = vec![
        html! { p.c { "\" :colorscheme: " (themes.len()) " palettes, * is yours" } },
        html! { p.c { "\" j k previews · Enter or click keeps it · leaving without Enter reverts" } },
        html! { p {} },
    ];
    for candidate in themes {
        let mark = if candidate.slug == theme.slug {
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
    layout(
        Page {
            site,
            tree,
            theme,
            path: "/theme",
            file: "[colorscheme]",
            filetype: "colors",
            title: &tree.title("theme"),
            description: &tree.site.descriptions.themes,
            json_ld: None,
        },
        lines,
    )
}

pub fn help(site: &Site, tree: &Tree<'_>, theme: &Theme) -> Markup {
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
        html! { h1.h1 { "                    " (tree.site.title.to_uppercase()) " · THE MANUAL" } },
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
            title: &tree.title("help"),
            description: &format!(
                "Keys and commands for {}. None of them are required.",
                tree.site.title
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
    /// Every project indexed, drafts included.
    pub projects: i64,
    pub themes: usize,
}

/// `:checkhealth` as a page: rendered on the server from the state the
/// private API reports, so it needs no script and shows what a visitor gets.
pub fn health(site: &Site, tree: &Tree<'_>, report: &Health<'_>, theme: &Theme) -> Markup {
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
    let mut lines = vec![
        html! { h1.h1 { "hldr: require(\"hldr.health\").check()" } },
        html! { p {} },
        html! { h2.h2 { span.mk { "## " } "server" } },
        ok(html! { (version) (built) }),
        ok(html! {
            "database answers; " (count(report.projects, "project")) " and "
            (count(report.themes as i64, "theme")) " indexed"
        }),
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
        ok(html! { "colorscheme " a href="/theme" { (theme.slug) } }),
        ok(html! { "javascript: optional; every page is a URL" }),
    ]);
    layout(
        Page {
            site,
            tree,
            theme,
            path: "/health",
            file: "[checkhealth]",
            filetype: "checkhealth",
            title: &tree.title("checkhealth"),
            description: "Server version, content revision and sync state.",
            json_ld: None,
        },
        lines,
    )
}

pub fn not_found(site: &Site, tree: &Tree<'_>, path: &str, detail: &str, theme: &Theme) -> Markup {
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
            title: &tree.title("404"),
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

fn count(n: i64, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
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
