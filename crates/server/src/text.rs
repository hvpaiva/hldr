use std::fmt::Write as _;

use hldr_core::{Profile, Project, ProjectSummary};

use crate::Site;
use crate::html::display_url;
use crate::theme::Theme;

pub struct Tty {
    color: bool,
}

impl Tty {
    pub fn new(color: bool) -> Self {
        Self { color }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    fn green(&self, text: &str) -> String {
        self.paint("1;32", text)
    }

    fn yellow(&self, text: &str) -> String {
        self.paint("1;33", text)
    }

    fn blue(&self, text: &str) -> String {
        self.paint("34", text)
    }

    fn cyan(&self, text: &str) -> String {
        self.paint("36", text)
    }

    fn gray(&self, text: &str) -> String {
        self.paint("90", text)
    }

    fn white(&self, text: &str) -> String {
        self.paint("37", text)
    }
}

const ASCII: &str = "\
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

pub fn home(site: &Site, profile: &Profile, projects: &[ProjectSummary], color: bool) -> String {
    let t = Tty::new(color);
    let mut out = String::new();
    prompt(&mut out, &t, site, "/");
    writeln!(out).ok();
    for line in ASCII.lines() {
        if line.is_empty() {
            writeln!(out).ok();
        } else {
            writeln!(out, "  {}", t.green(line)).ok();
        }
    }
    writeln!(out).ok();
    writeln!(out, "  {}", t.yellow(&profile.name)).ok();
    writeln!(out, "  {}", t.cyan(&profile.headline)).ok();
    if let Some(src) = profile
        .links
        .source
        .as_deref()
        .or(profile.links.github.as_deref())
    {
        writeln!(out, "  src: {}", t.blue(display_url(src))).ok();
    }
    writeln!(out).ok();
    tui_box(&mut out, &t, "About", &wrap(&profile.bio, BOX_INNER - 1));
    writeln!(out).ok();
    writeln!(out, "  {} {}", t.yellow("~"), t.green("Projects")).ok();
    write_projects(&mut out, &t, projects);
    writeln!(out).ok();
    writeln!(out, "  {} {}", t.yellow(">"), t.green("Elsewhere")).ok();
    write_elsewhere(&mut out, &t, profile);
    writeln!(out).ok();
    writeln!(out, "  {} {}", t.yellow(">"), t.green("Try it")).ok();
    write_try(
        &mut out,
        &t,
        &format!("{}/projects", site.host),
        "list projects",
    );
    write_try(&mut out, &t, &format!("{}/about", site.host), "about");
    writeln!(out).ok();
    writeln!(out, "{} {}", t.green("$"), t.gray("█")).ok();
    out
}

pub fn projects_index(site: &Site, projects: &[ProjectSummary], color: bool) -> String {
    let t = Tty::new(color);
    let mut out = String::new();
    prompt(&mut out, &t, site, "/projects");
    writeln!(out).ok();
    writeln!(out, "  {}", t.green("Projects")).ok();
    writeln!(out).ok();
    write_projects(&mut out, &t, projects);
    writeln!(out).ok();
    writeln!(out, "{} {}", t.green("$"), t.gray("█")).ok();
    out
}

pub fn themes_index(site: &Site, current: &Theme, color: bool) -> String {
    let t = Tty::new(color);
    let mut out = String::new();
    let prev = current.prev();
    let next = current.next();
    let n = current.index() + 1;
    let total = crate::theme::THEMES.len();
    prompt(&mut out, &t, site, "/theme");
    writeln!(out).ok();
    writeln!(out, "  {}", t.green("Themes")).ok();
    writeln!(out).ok();
    writeln!(out, "    {} {}", t.gray("←"), t.white(prev.name)).ok();
    writeln!(out, "  {} {}", t.yellow("*"), t.green(current.name)).ok();
    writeln!(out, "    {} {}", t.gray("→"), t.white(next.name)).ok();
    writeln!(out).ok();
    writeln!(out, "  {}", t.gray(&format!("{n} / {total}"))).ok();
    writeln!(out).ok();
    writeln!(out, "{} {}", t.green("$"), t.gray("█")).ok();
    out
}

pub fn project_page(site: &Site, project: &Project, color: bool) -> String {
    let t = Tty::new(color);
    let mut out = String::new();
    let path = format!("/projects/{}", project.slug);
    prompt(&mut out, &t, site, &path);
    writeln!(out).ok();
    let mut box_lines = wrap(&project.tagline, BOX_INNER - 1);
    let mut meta = project.status.as_str().to_owned();
    if !project.tags.is_empty() {
        meta.push_str(" · ");
        meta.push_str(&project.tags.join(", "));
    }
    box_lines.push(String::new());
    box_lines.push(meta);
    tui_box(&mut out, &t, &project.title, &box_lines);
    writeln!(out).ok();
    for line in project.body_text.lines() {
        writeln!(out, "  {line}").ok();
    }
    if let Some(repo) = &project.links.repo {
        writeln!(out).ok();
        writeln!(
            out,
            "  {} {} {}",
            t.green("$"),
            t.white("git clone"),
            t.blue(display_url(repo))
        )
        .ok();
    }
    writeln!(out).ok();
    writeln!(out, "{} {}", t.green("$"), t.gray("█")).ok();
    out
}

pub fn about(site: &Site, profile: &Profile, color: bool) -> String {
    let t = Tty::new(color);
    let mut out = String::new();
    prompt(&mut out, &t, site, "/about");
    writeln!(out).ok();
    let mut about_lines = vec![profile.headline.clone()];
    if !profile.about_text.trim().is_empty() {
        about_lines.push(String::new());
        about_lines.extend(wrap(&profile.about_text, BOX_INNER - 1));
    }
    tui_box(&mut out, &t, &profile.name, &about_lines);
    writeln!(out).ok();
    write_elsewhere(&mut out, &t, profile);
    writeln!(out).ok();
    writeln!(out, "{} {}", t.green("$"), t.gray("█")).ok();
    out
}

pub fn not_found(site: &Site, color: bool, detail: &str) -> String {
    let t = Tty::new(color);
    let mut out = String::new();
    writeln!(out).ok();
    writeln!(out, "  {}", t.yellow("┌─┐ ┌─┐ ┌─┐")).ok();
    writeln!(out, "  {}", t.yellow("│4│ │0│ │4│")).ok();
    writeln!(out, "  {}", t.yellow("└─┘ └─┘ └─┘")).ok();
    writeln!(out).ok();
    writeln!(out, "  {}", t.gray(detail)).ok();
    writeln!(out).ok();
    writeln!(
        out,
        "  {} {} {}",
        t.green("$"),
        t.white("curl"),
        t.blue(&site.host)
    )
    .ok();
    writeln!(
        out,
        "  {} {} {}",
        t.green("$"),
        t.white("curl"),
        t.blue(&format!("{}/projects", site.host))
    )
    .ok();
    writeln!(out).ok();
    writeln!(out, "{} {}", t.green("$"), t.gray("█")).ok();
    out
}

fn prompt(out: &mut String, t: &Tty, site: &Site, path: &str) {
    let path = if path == "/" { "" } else { path };
    writeln!(
        out,
        "\n{} {} {}",
        t.green("$"),
        t.white("curl"),
        t.blue(&format!("{}{path}", site.host))
    )
    .ok();
}

const BOX_INNER: usize = 64;
const BOX_WIDTH: usize = BOX_INNER + 2;
const COL_NUM: usize = 2;
const COL_NAME: usize = 16;

fn tui_box(out: &mut String, t: &Tty, title: &str, lines: &[String]) {
    let fill = BOX_WIDTH.saturating_sub(5 + visible_width(title));
    writeln!(
        out,
        "  {}{}{}{}{}",
        t.gray("┌─ "),
        t.yellow(title),
        t.gray(" "),
        t.gray(&"─".repeat(fill)),
        t.gray("┐")
    )
    .ok();
    for line in lines {
        let body = pad_right(&truncate(line, BOX_INNER - 1), BOX_INNER - 1);
        writeln!(out, "  {} {}{}", t.gray("│"), body, t.gray("│")).ok();
    }
    writeln!(
        out,
        "  {}{}{}",
        t.gray("└"),
        t.gray(&"─".repeat(BOX_WIDTH.saturating_sub(2))),
        t.gray("┘")
    )
    .ok();
}

fn write_projects(out: &mut String, t: &Tty, projects: &[ProjectSummary]) {
    if projects.is_empty() {
        writeln!(out, "    {}", t.gray("no projects yet.")).ok();
        return;
    }
    writeln!(
        out,
        "  {}  {}  {}",
        pad_right(&t.cyan("#"), COL_NUM),
        pad_right(&t.cyan("name"), COL_NAME),
        t.cyan("description")
    )
    .ok();
    for (i, project) in projects.iter().enumerate() {
        writeln!(
            out,
            "  {}  {}  {}",
            pad_right(&t.yellow(&format!("{}", i + 1)), COL_NUM),
            pad_right(&t.white(&truncate(&project.title, COL_NAME)), COL_NAME),
            t.gray(&project.tagline)
        )
        .ok();
    }
}

fn write_elsewhere(out: &mut String, t: &Tty, profile: &Profile) {
    const KEY_W: usize = 10;
    if let Some(github) = &profile.links.github {
        writeln!(
            out,
            "    {}  {}",
            pad_right(&t.cyan("github"), KEY_W),
            t.blue(display_url(github))
        )
        .ok();
    }
    if let Some(linkedin) = &profile.links.linkedin {
        writeln!(
            out,
            "    {}  {}",
            pad_right(&t.cyan("linkedin"), KEY_W),
            t.blue(display_url(linkedin))
        )
        .ok();
    }
    if let Some(email) = &profile.email {
        writeln!(
            out,
            "    {}  {}",
            pad_right(&t.cyan("mail"), KEY_W),
            t.blue(email)
        )
        .ok();
    }
}

fn write_try(out: &mut String, t: &Tty, host_path: &str, comment: &str) {
    let cmd = format!(
        "    {} {} {}",
        t.green("$"),
        t.white("curl"),
        t.blue(host_path)
    );
    writeln!(
        out,
        "{}  {}",
        pad_right(&cmd, 42),
        t.gray(&format!("# {comment}"))
    )
    .ok();
}

fn visible_width(s: &str) -> usize {
    let mut n = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        n += 1;
    }
    n
}

fn pad_right(s: &str, width: usize) -> String {
    let n = visible_width(s);
    if n >= width {
        s.to_owned()
    } else {
        format!("{s}{}", " ".repeat(width - n))
    }
}

fn truncate(s: &str, width: usize) -> String {
    let n = visible_width(s);
    if n <= width {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    let mut shown = 0;
    let keep = width.saturating_sub(1);
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            out.push(c);
            if let Some(bracket) = chars.next() {
                out.push(bracket);
            }
            for c in chars.by_ref() {
                out.push(c);
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        if shown >= keep {
            out.push('…');
            break;
        }
        out.push(c);
        shown += 1;
    }
    out
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split("\n\n") {
        let para = para.split_whitespace().collect::<Vec<_>>().join(" ");
        if para.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        wrap_words(&para, width, &mut lines);
    }
    lines
}

fn wrap_words(text: &str, width: usize, lines: &mut Vec<String>) {
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        } else {
            current.push(' ');
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Site;
    use hldr_core::Profile;
    use hldr_core::types::{ProfileLinks, ProjectLinks, ProjectStatus};

    fn site() -> Site {
        Site {
            origin: "https://hvpaiva.dev".into(),
            host: "hvpaiva.dev".into(),
        }
    }

    fn profile() -> Profile {
        Profile {
            name: "Test".into(),
            headline: "eng".into(),
            bio: "bio".into(),
            about_source: String::new(),
            about_html: String::new(),
            about_text: "about body".into(),
            email: None,
            links: ProfileLinks::default(),
            updated_at: String::new(),
        }
    }

    fn project() -> ProjectSummary {
        ProjectSummary {
            slug: "hldr".into(),
            title: "hldr".into(),
            tagline: "Site and CLI".into(),
            status: ProjectStatus::Active,
            highlight: Some(1),
            tags: vec!["rust".into()],
            links: ProjectLinks::default(),
            github_repo: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn color_uses_ansi_16_not_truecolor() {
        let out = home(&site(), &profile(), &[], true);
        assert!(out.contains("\x1b[1;32m"), "prompt should use ANSI green");
        assert!(
            !out.contains("\x1b[38;2;"),
            "truecolor would override the terminal theme"
        );
        assert!(
            !out.contains("\x1b[38;5;"),
            "256-color would override the terminal theme"
        );
    }

    #[test]
    fn visible_width_skips_ansi() {
        assert_eq!(visible_width("hldr"), 4);
        assert_eq!(visible_width("\x1b[37mhldr\x1b[0m"), 4);
    }

    #[test]
    fn project_columns_left_align() {
        let out = projects_index(&site(), &[project()], false);
        let header = out
            .lines()
            .find(|line| line.contains("name") && line.contains("description"))
            .expect("header");
        let row = out
            .lines()
            .find(|line| line.contains("hldr") && line.contains("Site"))
            .expect("row");
        assert_eq!(header.find("name"), row.find("hldr"));
        assert_eq!(header.find("description"), row.find("Site"));
    }

    #[test]
    fn home_box_and_no_web_affordance() {
        let out = home(&site(), &profile(), &[], false);
        assert!(out.contains("┌─ "));
        assert!(out.contains("└"));
        assert!(!out.contains("view all"));
        assert!(!out.contains("this page"));
    }

    #[test]
    fn wrap_reflows_soft_breaks_and_keeps_paragraphs() {
        let lines = wrap(
            "pipelines that are\nboring, hosts that can be rebuilt.\n\nNext paragraph.",
            40,
        );
        assert_eq!(
            lines,
            [
                "pipelines that are boring, hosts that",
                "can be rebuilt.",
                "",
                "Next paragraph.",
            ]
        );
    }

    #[test]
    fn about_is_a_closed_box() {
        let out = about(&site(), &profile(), false);
        assert!(out.contains("┌─ "));
        assert!(out.contains("about body"));
        assert!(out.contains("└"));
    }
}
