//! Renders content as the source an editor would show: one element per
//! line, syntax left visible and coloured, links still clickable.

use hldr_core::template::{self, Block, Segment};
use maud::{Markup, html};

pub use hldr_core::template::safe_url;

/// What the directives in a page's markdown render: values inside a line,
/// and whole lines for a block directive.
pub trait Expand {
    /// The text a value path names, as a template inserts it.
    fn value(&self, path: &str) -> String;
    /// The lines a block directive renders; none is fine.
    fn block(&self, block: &Block) -> Vec<Markup>;
}

/// Markdown source, one element per line. Headings become real heading
/// elements so the page keeps its outline for readers and crawlers. A block
/// directive alone on its line renders the lines it stands for, or none.
pub fn markdown(source: &str, expand: &dyn Expand) -> Vec<Markup> {
    let mut lines = Vec::new();
    let mut fence: Option<&str> = None;
    for raw in source.lines() {
        let trimmed = raw.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
                lines.push(html! { p.mk { (raw) } });
            } else {
                lines.push(html! { p.code { (raw) } });
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(&trimmed[..3]);
            lines.push(html! { p.mk { (raw) } });
            continue;
        }
        if let Ok(Some(block)) = template::block(raw) {
            lines.extend(expand.block(&block));
            continue;
        }
        lines.push(markdown_line(raw, expand));
    }
    lines
}

/// The block structure comes from the line as written, before any value is
/// inserted, so a value can never turn into a heading or a list.
fn markdown_line(raw: &str, expand: &dyn Expand) -> Markup {
    if raw.trim().is_empty() {
        return html! { p {} };
    }
    if let Some((level, text)) = heading(raw) {
        let marker = format!("{} ", "#".repeat(level));
        let text = expanded(text, expand);
        return match level {
            1 => html! { h1.h1 { span.mk { (marker) } (text) } },
            2 => html! { h2.h2 { span.mk { (marker) } (text) } },
            _ => html! { h3.h2 { span.mk { (marker) } (text) } },
        };
    }
    if let Some(text) = raw.strip_prefix("> ") {
        return html! { p.q { span.mk { "> " } (expanded(text, expand)) } };
    }
    if let Some((indent, marker, text)) = list_item(raw) {
        return html! { p { (indent) span.mk { (marker) } (expanded(text, expand)) } };
    }
    html! { p { (expanded(raw, expand)) } }
}

/// Inline markdown with its inline directives rendered: values as inline
/// markdown, links without brackets. Markdown markers do not reach across a
/// directive. A line that does not parse, which the indexer never stores,
/// renders as written.
pub fn expanded(text: &str, expand: &dyn Expand) -> Markup {
    let Ok(parts) = template::segments(text) else {
        return inline(text);
    };
    html! {
        @for part in &parts {
            @match part {
                Segment::Text(text) => (inline(text)),
                Segment::Value(path) => (inline(&expand.value(path))),
                Segment::Link { href, label } => {
                    @match safe_url(href) {
                        Some(href) => a href=(href) { (label) },
                        None => (label),
                    }
                }
            }
        }
    }
}

fn heading(raw: &str) -> Option<(usize, &str)> {
    let level = raw.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    raw[level..].strip_prefix(' ').map(|text| (level, text))
}

fn list_item(raw: &str) -> Option<(&str, String, &str)> {
    let indent_len = raw.len() - raw.trim_start().len();
    let (indent, rest) = raw.split_at(indent_len);
    for bullet in ["- ", "* ", "+ "] {
        if let Some(text) = rest.strip_prefix(bullet) {
            return Some((indent, bullet.to_owned(), text));
        }
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0
        && let Some(text) = rest[digits..].strip_prefix(". ")
    {
        return Some((indent, format!("{}. ", &rest[..digits]), text));
    }
    None
}

/// Inline markdown: `code`, **strong**, [text](url) and bare URLs.
/// Markers stay visible; only safe URLs become links.
pub fn inline(text: &str) -> Markup {
    let mut parts: Vec<Markup> = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            flush(&mut parts, &mut plain);
            let code = &after[..end];
            parts.push(html! { span.s { "`" (code) "`" } });
            rest = &after[end + 1..];
            continue;
        }
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            flush(&mut parts, &mut plain);
            let strong = &after[..end];
            parts.push(html! { span.mk { "**" } strong { (strong) } span.mk { "**" } });
            rest = &after[end + 2..];
            continue;
        }
        if let Some((label, url, used)) = link(rest) {
            flush(&mut parts, &mut plain);
            parts.push(match safe_url(url) {
                Some(href) => html! { span.mk { "[" } a href=(href) { (label) } span.mk { "]" } },
                None => html! { (&rest[..used]) },
            });
            rest = &rest[used..];
            continue;
        }
        if rest.starts_with("https://") || rest.starts_with("http://") {
            flush(&mut parts, &mut plain);
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ')' || c == '>')
                .unwrap_or(rest.len());
            let url = rest[..end].trim_end_matches(['.', ',', ';', ':']);
            parts.push(html! { a href=(url) { (url) } });
            rest = &rest[url.len()..];
            continue;
        }
        let ch = rest.chars().next().unwrap_or_default();
        plain.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    flush(&mut parts, &mut plain);
    html! { @for part in &parts { (part) } }
}

fn flush(parts: &mut Vec<Markup>, plain: &mut String) {
    if !plain.is_empty() {
        let text = std::mem::take(plain);
        parts.push(html! { (text) });
    }
}

fn link(rest: &str) -> Option<(&str, &str, usize)> {
    let after = rest.strip_prefix('[')?;
    let close = after.find("](")?;
    let label = &after[..close];
    if label.contains(['[', ']']) {
        return None;
    }
    let tail = &after[close + 2..];
    let end = tail.find(')')?;
    let url = &tail[..end];
    Some((label, url, 1 + close + 2 + end + 1))
}

/// A YAML `key: value` line, with the value already rendered.
pub fn yaml_pair(indent: usize, key: &str, value: Markup) -> Markup {
    html! {
        p { (" ".repeat(indent)) span.k { (key) } span.mk { ":" } " " (value) }
    }
}

/// A YAML key that opens a nested block.
pub fn yaml_key(indent: usize, key: &str) -> Markup {
    html! { p { (" ".repeat(indent)) span.k { (key) } span.mk { ":" } } }
}

/// Plain YAML scalar in the string colour.
pub fn yaml_str(value: &str) -> Markup {
    html! { span.s { (value) } }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values are their path in capitals; blocks render one marked line.
    struct Stub;

    impl Expand for Stub {
        fn value(&self, path: &str) -> String {
            path.to_uppercase()
        }

        fn block(&self, block: &Block) -> Vec<Markup> {
            match block {
                Block::Links { .. } => Vec::new(),
                other => vec![html! { p.block { (format!("{other:?}")) } }],
            }
        }
    }

    fn markdown(source: &str) -> Vec<Markup> {
        super::markdown(source, &Stub)
    }

    fn render(lines: &[Markup]) -> Vec<String> {
        lines.iter().map(|line| line.0.clone()).collect()
    }

    #[test]
    fn one_element_per_source_line() {
        let src = "## What it is\n\nPersonal site.\nSecond line.\n";
        let out = render(&markdown(src));
        assert_eq!(out.len(), 4);
        assert_eq!(
            out[0],
            r#"<h2 class="h2"><span class="mk">## </span>What it is</h2>"#
        );
        assert_eq!(out[1], "<p></p>");
    }

    #[test]
    fn fences_are_verbatim() {
        let src = "```sh\ncurl **x** [a](b)\n```\n";
        let out = render(&markdown(src));
        assert_eq!(out[1], r#"<p class="code">curl **x** [a](b)</p>"#);
        assert!(out[2].contains("```"));
    }

    #[test]
    fn list_markers_and_quotes_stay_visible() {
        let out = render(&markdown("- one\n  2. two\n> said\n"));
        assert!(out[0].contains(r#"<span class="mk">- </span>one"#));
        assert!(out[1].starts_with("<p>  <span class=\"mk\">2. </span>two"));
        assert!(out[2].contains(r#"<span class="mk">&gt; </span>said"#));
    }

    #[test]
    fn inline_code_strong_and_links() {
        let out = inline("run `curl x` **now**, see [docs](https://x.dev/a).").0;
        assert!(out.contains(r#"<span class="s">`curl x`</span>"#));
        assert!(out.contains("<strong>now</strong>"));
        assert!(out.contains(r#"<a href="https://x.dev/a">docs</a>"#));
        assert!(out.ends_with("."));
    }

    #[test]
    fn bare_urls_drop_trailing_punctuation() {
        let out = inline("at https://github.com/hvpaiva/hldr.").0;
        assert!(out.contains(r#"<a href="https://github.com/hvpaiva/hldr">"#));
        assert!(out.ends_with("</a>."));
    }

    #[test]
    fn unsafe_links_render_as_text() {
        let out = inline("[click](javascript:alert(1))").0;
        assert!(!out.contains("<a"));
        assert!(out.contains("javascript:alert(1)"));
        assert_eq!(safe_url("//evil.example"), None);
        assert_eq!(safe_url("/projects"), Some("/projects"));
        assert_eq!(safe_url("mailto:a@b.c"), Some("mailto:a@b.c"));
    }

    #[test]
    fn markup_is_escaped() {
        let out = render(&markdown("<script>alert(1)</script>\n# <b>x</b>\n"));
        assert!(!out.concat().contains("<script>"));
        assert!(out[0].contains("&lt;script&gt;"));
        assert!(out[1].contains("&lt;b&gt;x&lt;/b&gt;"));
    }

    #[test]
    fn not_a_heading_without_space() {
        assert!(heading("#hashtag").is_none());
        assert!(heading("####### seven").is_none());
        assert_eq!(heading("### three"), Some((3, "three")));
    }

    #[test]
    fn directives_render_in_place() {
        let out = render(&markdown(
            "# {{ profile.name }}\n> {{ profile.headline }}\n## {{ link /projects Projects }}\n{{ site.values.about }} More in {{ link /about about.md }}.\n",
        ));
        assert_eq!(
            out[0],
            r#"<h1 class="h1"><span class="mk"># </span>PROFILE.NAME</h1>"#
        );
        assert_eq!(
            out[1],
            r#"<p class="q"><span class="mk">&gt; </span>PROFILE.HEADLINE</p>"#
        );
        assert_eq!(
            out[2],
            r#"<h2 class="h2"><span class="mk">## </span><a href="/projects">Projects</a></h2>"#
        );
        assert_eq!(
            out[3],
            r#"<p>SITE.VALUES.ABOUT More in <a href="/about">about.md</a>.</p>"#
        );
    }

    #[test]
    fn block_directives_take_their_line_or_none() {
        let out = render(&markdown(
            "a\n{{ links }}\n{{ clone page.links.repo }}\nb\n",
        ));
        assert_eq!(out.len(), 3, "{out:?}");
        assert!(out[1].contains("Clone"), "{out:?}");
    }

    #[test]
    fn nothing_expands_in_code() {
        let out = render(&markdown(
            "```\n{{ profile.name }}\n```\nsay `{{ profile.name }}`\n",
        ));
        assert_eq!(out[1], r#"<p class="code">{{ profile.name }}</p>"#);
        assert_eq!(
            out[3],
            r#"<p>say <span class="s">`{{ profile.name }}`</span></p>"#
        );
    }

    #[test]
    fn values_are_inline_markdown_but_never_blocks() {
        struct Heading;
        impl Expand for Heading {
            fn value(&self, _: &str) -> String {
                "# not a heading, **strong** though".to_owned()
            }
            fn block(&self, _: &Block) -> Vec<Markup> {
                Vec::new()
            }
        }
        let out = render(&super::markdown("{{ site.values.x }}\n", &Heading));
        assert_eq!(
            out[0],
            r#"<p># not a heading, <span class="mk">**</span><strong>strong</strong><span class="mk">**</span> though</p>"#
        );
    }
}
