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
    rule(&mut out, &t, "About");
    for line in wrap(&profile.bio, 76) {
        writeln!(out, "  {line}").ok();
    }
    writeln!(out, "  {}", t.gray(&"─".repeat(68))).ok();
    writeln!(out).ok();
    writeln!(
        out,
        "  {} {}                    {}",
        t.yellow("~"),
        t.green("Projects"),
        t.gray("view all →")
    )
    .ok();
    write_projects(&mut out, &t, projects);
    writeln!(out).ok();
    writeln!(out, "  {} {}", t.yellow(">"), t.green("Elsewhere")).ok();
    write_elsewhere(&mut out, &t, profile);
    writeln!(out).ok();
    writeln!(out, "  {} {}", t.yellow(">"), t.green("Try it")).ok();
    writeln!(
        out,
        "    {} {} {}          {}",
        t.green("$"),
        t.white("curl"),
        t.blue(&site.host),
        t.gray("this page")
    )
    .ok();
    writeln!(
        out,
        "    {} {} {} {}",
        t.green("$"),
        t.white("curl"),
        t.blue(&format!("{}/projects", site.host)),
        t.gray("list projects")
    )
    .ok();
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
    rule(&mut out, &t, &project.title);
    writeln!(out, "  {}", project.tagline).ok();
    let mut meta = project.status.as_str().to_owned();
    if !project.tags.is_empty() {
        meta.push_str(" · ");
        meta.push_str(&project.tags.join(", "));
    }
    writeln!(out, "  {}", t.gray(&meta)).ok();
    writeln!(out, "  {}", t.gray(&"─".repeat(68))).ok();
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
    writeln!(out, "  {}", t.yellow(&profile.name)).ok();
    writeln!(out, "  {}", t.cyan(&profile.headline)).ok();
    writeln!(out).ok();
    for line in profile.about_text.lines() {
        writeln!(out, "  {line}").ok();
    }
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

fn rule(out: &mut String, t: &Tty, title: &str) {
    let fill = 60usize.saturating_sub(title.len());
    writeln!(
        out,
        "  {} {} {}",
        t.gray("──"),
        t.yellow(title),
        t.gray(&"─".repeat(fill))
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
        "    {}   {}                      {}",
        t.cyan("#"),
        t.cyan("name"),
        t.cyan("description")
    )
    .ok();
    for (i, project) in projects.iter().enumerate() {
        writeln!(
            out,
            "  {:>2} {:<22} {}",
            t.yellow(&format!("{}", i + 1)),
            t.white(&project.title),
            t.gray(&project.tagline)
        )
        .ok();
    }
}

fn write_elsewhere(out: &mut String, t: &Tty, profile: &Profile) {
    if let Some(github) = &profile.links.github {
        writeln!(
            out,
            "    {} {}   {}",
            t.green("$"),
            t.white("gh"),
            t.blue(display_url(github))
        )
        .ok();
    }
    if let Some(linkedin) = &profile.links.linkedin {
        writeln!(
            out,
            "    {} {}   {}",
            t.green("$"),
            t.white("in"),
            t.blue(display_url(linkedin))
        )
        .ok();
    }
    if let Some(email) = &profile.email {
        writeln!(
            out,
            "    {} {} {}",
            t.green("$"),
            t.white("mail"),
            t.blue(email)
        )
        .ok();
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let para = para.trim();
        if para.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in para.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.len() + 1 + word.len() > width {
                lines.push(current);
                current = word.to_owned();
            } else {
                current.push(' ');
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Site;
    use hldr_core::Profile;
    use hldr_core::types::ProfileLinks;

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
            about_text: String::new(),
            email: None,
            links: ProfileLinks::default(),
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
}
