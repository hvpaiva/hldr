//! Renders content as the source an editor would show: one element per
//! line, syntax left visible and coloured, links still clickable.

use maud::{Markup, html};

/// Markdown source, one element per line. Headings become real heading
/// elements so the page keeps its outline for readers and crawlers.
pub fn markdown(source: &str) -> Vec<Markup> {
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
        lines.push(markdown_line(raw));
    }
    lines
}

fn markdown_line(raw: &str) -> Markup {
    if raw.trim().is_empty() {
        return html! { p {} };
    }
    if let Some((level, text)) = heading(raw) {
        let marker = format!("{} ", "#".repeat(level));
        return match level {
            1 => html! { h1.h1 { span.mk { (marker) } (inline(text)) } },
            2 => html! { h2.h2 { span.mk { (marker) } (inline(text)) } },
            _ => html! { h3.h2 { span.mk { (marker) } (inline(text)) } },
        };
    }
    if let Some(text) = raw.strip_prefix("> ") {
        return html! { p.q { span.mk { "> " } (inline(text)) } };
    }
    if let Some((indent, marker, text)) = list_item(raw) {
        return html! { p { (indent) span.mk { (marker) } (inline(text)) } };
    }
    html! { p { (inline(raw)) } }
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

/// Allows http(s), mailto and same-site paths. Anything else, such as
/// `javascript:`, is rendered as text instead of a link.
pub fn safe_url(url: &str) -> Option<&str> {
    let lower = url.trim().to_ascii_lowercase();
    let ok = lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || (lower.starts_with('/') && !lower.starts_with("//"))
        || lower.starts_with('#');
    ok.then_some(url.trim())
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

/// Wraps prose at `width` columns without splitting words.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let fits =
            current.is_empty() || current.chars().count() + 1 + word.chars().count() <= width;
        if !fits {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn wrap_respects_width() {
        let lines = wrap("aaa bbb ccc ddd", 7);
        assert_eq!(lines, vec!["aaa bbb", "ccc ddd"]);
        assert!(wrap("", 10).is_empty());
    }
}
