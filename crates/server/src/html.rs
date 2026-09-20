use hldr_core::{Profile, Project, ProjectSummary};
use maud::{DOCTYPE, Markup, PreEscaped, html};

use crate::Site;
use crate::theme::Theme;

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Home,
    Projects,
    About,
    Theme,
    None,
}

pub struct Page<'a> {
    pub site: &'a Site,
    pub nav: Nav,
    pub path: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub project_count: i64,
    pub json_ld: Option<String>,
    pub theme: &'static Theme,
}

pub fn layout(page: Page<'_>, body: Markup) -> Markup {
    let canonical = format!("{}{}", page.site.origin, page.path);
    let description = oneline(page.description);
    html! {
        (DOCTYPE)
        html lang="en" data-theme=(page.theme.slug) {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (page.title) }
                meta name="description" content=(description);
                link rel="canonical" href=(canonical);
                meta property="og:title" content=(page.title);
                meta property="og:description" content=(description);
                meta property="og:url" content=(canonical);
                meta property="og:type" content="website";
                meta name="twitter:card" content="summary";
                link rel="icon" href=(format!("/favicon.svg?t={}", page.theme.slug)) type="image/svg+xml";
                link rel="stylesheet" href="/style.css?v=keys";
                style { (PreEscaped(page.theme.root_css())) }
                @if let Some(json_ld) = &page.json_ld {
                    script type="application/ld+json" { (PreEscaped(json_ld)) }
                }
            }
            body {
                a.skip-link href="#main-content" { "skip to content" }
                header.title-bar role="banner" {
                    div.title-bar-inner {
                        a.term-title href="/" { (page.site.host) }
                        div.title-bar-end {
                            nav.title-bar-nav aria-label="Main navigation" {
                                (nav_link("/", "~", page.nav == Nav::Home))
                                (nav_link("/projects", "projects", page.nav == Nav::Projects))
                                (nav_link("/about", "about", page.nav == Nav::About))
                            }
                            (theme_link(page.theme, page.nav == Nav::Theme))
                            (keys_help())
                        }
                    }
                }
                main #main-content .container {
                    (body)
                }
                footer.status-bar role="contentinfo" {
                    div.status-bar-inner {
                        span.status-l { "NORMAL  │  " (page.path) }
                        span.status-r {
                            (page.project_count)
                            @if page.project_count == 1 { " project" } @else { " projects" }
                            "  │  hldr "
                            (hldr_core::VERSION)
                        }
                    }
                }
                script src="/keys.js" defer {}
            }
        }
    }
}

pub fn home(
    site: &Site,
    profile: &Profile,
    projects: &[ProjectSummary],
    count: i64,
    theme: &'static Theme,
) -> Markup {
    let json_ld = person_ld(site, profile);
    let src = profile
        .links
        .source
        .as_deref()
        .or(profile.links.github.as_deref());
    layout(
        Page {
            site,
            nav: Nav::Home,
            path: "/",
            title: &profile.name,
            description: &profile.bio,
            project_count: count,
            json_ld: Some(json_ld),
            theme,
        },
        html! {
            (prompt(site, "/"))
            div.response-block {
                section.hero {
                    div.ascii-wrap role="img" aria-label="HLDR" {
                        pre.ascii-banner aria-hidden="true" { (ASCII_BANNER) }
                    }
                    div.hero-info {
                        p.hero-subtitle { (profile.name) }
                        p.hero-tagline { (profile.headline) }
                        @if let Some(src) = src {
                            p.hero-link {
                                "src: "
                                a href=(src) rel="me" { (display_url(src)) }
                            }
                        }
                    }
                }
                fieldset.tui-box {
                    legend.tui-box-label { "About" }
                    @for para in paragraphs(&profile.bio) {
                        p { (para) }
                    }
                }
                section.home-section {
                    div.section-header {
                        span.section-icon { "~" }
                        h2.section-title {
                            "Projects"
                            a.section-link href="/projects" { "view all →" }
                        }
                    }
                    (project_table(projects))
                }
                section.home-section {
                    div.section-header {
                        span.section-icon { ">" }
                        h2.section-title { "Elsewhere" }
                    }
                    (elsewhere(profile))
                }
                section.home-section {
                    div.section-header {
                        span.section-icon { ">" }
                        h2.section-title { "Try it" }
                    }
                    table.try-table {
                        tr {
                            td { (curl_cmd(site, "")) }
                            td.try-desc { "this page" }
                        }
                        tr {
                            td { (curl_cmd(site, "/projects")) }
                            td.try-desc { "list projects" }
                        }
                        tr {
                            td { (curl_cmd(site, "/about")) }
                            td.try-desc { "about" }
                        }
                    }
                }
            }
            (page_cursor())
        },
    )
}

pub fn projects_index(
    site: &Site,
    projects: &[ProjectSummary],
    count: i64,
    description: &str,
    theme: &'static Theme,
) -> Markup {
    layout(
        Page {
            site,
            nav: Nav::Projects,
            path: "/projects",
            title: "projects · hvpaiva.dev",
            description,
            project_count: count,
            json_ld: None,
            theme,
        },
        html! {
            (prompt(site, "/projects"))
            div.response-block.list-page {
                h1.section-title { "Projects" }
                p.list-description { (description) }
                (project_table(projects))
            }
            (page_cursor())
        },
    )
}

pub fn project_page(site: &Site, project: &Project, count: i64, theme: &'static Theme) -> Markup {
    let json_ld = software_ld(site, project);
    let path = format!("/projects/{}", project.slug);
    let title = format!("{} · hvpaiva.dev", project.title);
    layout(
        Page {
            site,
            nav: Nav::Projects,
            path: &path,
            title: &title,
            description: &project.tagline,
            project_count: count,
            json_ld: Some(json_ld),
            theme,
        },
        html! {
            (prompt(site, &path))
            div.response-block {
                fieldset.tui-box {
                    legend.tui-box-label { (project.title) }
                    p { (project.tagline) }
                    p.post-meta {
                        (project.status.as_str())
                        @if !project.tags.is_empty() {
                            " · "
                            (project.tags.join(", "))
                        }
                    }
                }
                article.post-content {
                    (PreEscaped(project.body_html.as_str()))
                }
                @if has_links(project) {
                    section.home-section {
                        div.section-header {
                            span.section-icon { ">" }
                            h2.section-title { "Links" }
                        }
                        (project_links(project))
                    }
                }
                p.post-footer {
                    a href="/projects" { "← projects" }
                }
            }
            (page_cursor())
        },
    )
}

pub fn about(site: &Site, profile: &Profile, count: i64, theme: &'static Theme) -> Markup {
    layout(
        Page {
            site,
            nav: Nav::About,
            path: "/about",
            title: "about · hvpaiva.dev",
            description: &profile.headline,
            project_count: count,
            json_ld: Some(person_ld(site, profile)),
            theme,
        },
        html! {
            (prompt(site, "/about"))
            div.response-block {
                fieldset.tui-box {
                    legend.tui-box-label { (profile.name) }
                    p { (profile.headline) }
                }
                article.post-content {
                    (PreEscaped(profile.about_html.as_str()))
                }
                section.home-section {
                    div.section-header {
                        span.section-icon { ">" }
                        h2.section-title { "Contact" }
                    }
                    (elsewhere(profile))
                }
            }
            (page_cursor())
        },
    )
}

pub fn not_found(
    site: &Site,
    path: &str,
    count: i64,
    detail: &str,
    theme: &'static Theme,
) -> Markup {
    layout(
        Page {
            site,
            nav: Nav::None,
            path,
            title: "404 · hvpaiva.dev",
            description: detail,
            project_count: count,
            json_ld: None,
            theme,
        },
        html! {
            (prompt(site, path))
            div.response-block {
                pre.error-banner {
                    "┌─┐ ┌─┐ ┌─┐\n"
                    "│4│ │0│ │4│\n"
                    "└─┘ └─┘ └─┘"
                }
                p.error-msg { (detail) }
                nav.error-nav {
                    a href="/" { (curl_cmd(site, "")) } br;
                    a href="/projects" { (curl_cmd(site, "/projects")) }
                }
            }
            (page_cursor())
        },
    )
}

pub fn themes_index(
    site: &Site,
    profile: &Profile,
    projects: &[ProjectSummary],
    count: i64,
    theme: &'static Theme,
) -> Markup {
    let prev = theme.prev();
    let next = theme.next();
    let n = theme.index() + 1;
    let total = crate::theme::THEMES.len();
    layout(
        Page {
            site,
            nav: Nav::Theme,
            path: "/theme",
            title: "theme · hvpaiva.dev",
            description: "Pick a palette. Omarchy themes, applied to this page.",
            project_count: count,
            json_ld: None,
            theme,
        },
        html! {
            (prompt(site, "/theme"))
            div.response-block.list-page {
                div.theme-carousel {
                    (theme_slide(site, profile, projects, prev, false))
                    (theme_slide(site, profile, projects, theme, true))
                    (theme_slide(site, profile, projects, next, false))
                }
                nav.theme-cycle aria-label="Theme carousel" {
                    a.theme-cycle-arrow href=(format!("/theme/{}", prev.slug)) rel="prev" { "←" }
                    span.keys-hint { "h" }
                    h1.theme-cycle-name { (theme.name) }
                    span.keys-hint { "l" }
                    a.theme-cycle-arrow href=(format!("/theme/{}", next.slug)) rel="next" { "→" }
                }
                p.theme-count { (n) " / " (total) }
            }
            (page_cursor())
        },
    )
}

fn keys_help() -> Markup {
    html! {
        details.keys-help {
            summary.keys-help-toggle aria-label="Keyboard shortcuts" { "?" }
            div.keys-help-backdrop {
                fieldset.tui-box.keys-cheatsheet {
                    legend.tui-box-label { "keys" }
                    table.keys-table {
                        tr { td.keys-bind { "?" } td { "toggle this" } }
                        tr { td.keys-bind { "esc" } td { "close" } }
                        tr { td.keys-bind { "g h" } td { "home" } }
                        tr { td.keys-bind { "g p" } td { "projects" } }
                        tr { td.keys-bind { "g a" } td { "about" } }
                        tr { td.keys-bind { "g t" } td { "theme" } }
                        tr { td.keys-bind { "h / ←" } td { "previous theme" } }
                        tr { td.keys-bind { "l / →" } td { "next theme" } }
                        tr { td.keys-bind { "j k" } td { "move in list" } }
                        tr { td.keys-bind { "enter" } td { "open" } }
                    }
                }
            }
        }
    }
}

fn theme_link(theme: &Theme, active: bool) -> Markup {
    html! {
        @if active {
            a.theme-current href="/theme" aria-current="page" aria-label="Color theme" { (theme.name) }
        } @else {
            a.theme-current href="/theme" aria-label="Color theme" { (theme.name) }
        }
    }
}

fn theme_slide(
    site: &Site,
    profile: &Profile,
    projects: &[ProjectSummary],
    theme: &Theme,
    current: bool,
) -> Markup {
    let style = theme.preview_style();
    let class = if current {
        "theme-slide theme-slide-main"
    } else {
        "theme-slide theme-slide-side"
    };
    html! {
        @if current {
            div class=(class) style=(style) aria-current="true" {
                (mini_home(site, profile, projects))
            }
        } @else {
            a class=(class) href=(format!("/theme/{}", theme.slug)) style=(style) {
                (mini_home(site, profile, projects))
            }
        }
    }
}

fn mini_home(site: &Site, profile: &Profile, projects: &[ProjectSummary]) -> Markup {
    let preview_projects = projects.get(..1).unwrap_or(projects);
    html! {
        div.theme-preview {
            div.theme-preview-top {
                span { (site.host) }
                span { "~  projects  about" }
            }
            div.theme-preview-body {
                div.theme-preview-scale {
                    section.hero {
                        div.ascii-wrap role="presentation" {
                            pre.ascii-banner aria-hidden="true" { (ASCII_BANNER) }
                        }
                        div.hero-info {
                            p.hero-subtitle { (profile.name) }
                            p.hero-tagline { (profile.headline) }
                        }
                    }
                    fieldset.tui-box {
                        legend.tui-box-label { "About" }
                        @if let Some(para) = paragraphs(&profile.bio).first() {
                            p { (para) }
                        }
                    }
                    section.home-section {
                        div.section-header {
                            span.section-icon { "~" }
                            h2.section-title { "Projects" }
                        }
                        (project_table(preview_projects))
                    }
                }
            }
            div.theme-preview-foot {
                span { "NORMAL  │  /" }
                span { "hldr" }
            }
        }
    }
}

fn nav_link(href: &str, label: &str, active: bool) -> Markup {
    html! {
        @if active {
            a href=(href) class="active" aria-current="page" { (label) }
        } @else {
            a href=(href) { (label) }
        }
    }
}

fn project_table(projects: &[ProjectSummary]) -> Markup {
    if projects.is_empty() {
        return html! { p.empty-msg { "no projects yet." } };
    }
    html! {
        table.post-list {
            thead {
                tr {
                    th.th-num { "#" }
                    th { "name" }
                    th.col-desc { "description" }
                    th { "status" }
                    th.col-stack { "stack" }
                }
            }
            tbody {
                @for (i, project) in projects.iter().enumerate() {
                    tr.post-row {
                        td.post-num { (i + 1) }
                        td.post-title {
                            a href=(format!("/projects/{}", project.slug)) { (project.title) }
                        }
                        td.post-desc.col-desc { (project.tagline) }
                        td.post-meta-cell { (project.status.as_str()) }
                        td.post-meta-cell.col-stack {
                            @for tag in &project.tags {
                                span.tag { (tag) }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn elsewhere(profile: &Profile) -> Markup {
    html! {
        table.try-table {
            @if let Some(github) = &profile.links.github {
                tr {
                    td { span.sh-dollar { "$" } " " span.sh-cmd { "gh" } }
                    td { a href=(github) rel="me" { (display_url(github)) } }
                }
            }
            @if let Some(linkedin) = &profile.links.linkedin {
                tr {
                    td { span.sh-dollar { "$" } " " span.sh-cmd { "in" } }
                    td { a href=(linkedin) rel="me" { (display_url(linkedin)) } }
                }
            }
            @if let Some(email) = &profile.email {
                tr {
                    td { span.sh-dollar { "$" } " " span.sh-cmd { "mail" } }
                    td { a href=(format!("mailto:{email}")) { (email) } }
                }
            }
        }
    }
}

fn project_links(project: &Project) -> Markup {
    html! {
        table.try-table {
            @if let Some(repo) = &project.links.repo {
                tr {
                    td { span.sh-dollar { "$" } " " span.sh-cmd { "git clone" } }
                    td { a href=(repo) { (display_url(repo)) } }
                }
            }
            @if let Some(docs) = &project.links.docs {
                tr {
                    td { span.sh-dollar { "$" } " " span.sh-cmd { "docs" } }
                    td { a href=(docs) { (display_url(docs)) } }
                }
            }
            @if let Some(demo) = &project.links.demo {
                tr {
                    td { span.sh-dollar { "$" } " " span.sh-cmd { "demo" } }
                    td { a href=(demo) { (display_url(demo)) } }
                }
            }
        }
    }
}

fn has_links(project: &Project) -> bool {
    project.links.repo.is_some() || project.links.docs.is_some() || project.links.demo.is_some()
}

fn prompt(site: &Site, path: &str) -> Markup {
    html! {
        div.prompt-line { (curl_cmd(site, path.trim_end_matches('/'))) }
    }
}

fn curl_cmd(site: &Site, path: &str) -> Markup {
    let path = if path == "/" { "" } else { path };
    html! {
        span.sh-dollar { "$" }
        " "
        span.sh-cmd { "curl" }
        " "
        span.sh-url { (site.host) (path) }
    }
}

fn page_cursor() -> Markup {
    html! {
        div.page-cursor { span.sh-dollar { "$" } " " span.cursor {} }
    }
}

fn paragraphs(text: &str) -> Vec<&str> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|para| !para.is_empty())
        .collect()
}

pub fn display_url(url: &str) -> &str {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
}

fn oneline(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn person_ld(site: &Site, profile: &Profile) -> String {
    let mut same_as = Vec::new();
    if let Some(github) = &profile.links.github {
        same_as.push(github.clone());
    }
    if let Some(linkedin) = &profile.links.linkedin {
        same_as.push(linkedin.clone());
    }
    serde_json::json!({
        "@context": "https://schema.org",
        "@type": "Person",
        "name": profile.name,
        "jobTitle": profile.headline,
        "url": site.origin,
        "email": profile.email,
        "sameAs": same_as,
    })
    .to_string()
}

fn software_ld(site: &Site, project: &Project) -> String {
    serde_json::json!({
        "@context": "https://schema.org",
        "@type": "SoftwareSourceCode",
        "name": project.title,
        "description": project.tagline,
        "url": format!("{}/projects/{}", site.origin, project.slug),
        "codeRepository": project.links.repo,
        "programmingLanguage": project.tags,
    })
    .to_string()
}
