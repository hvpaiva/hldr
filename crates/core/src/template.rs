//! Templates in content: a closed set of directives, written `{{ name arg
//! key=value "quoted arg" }}`, in markdown and in a few string fields.
//!
//! This module only parses. The server renders what it parses; the tree
//! checks that every reference resolves. Nothing expands inside code, fenced
//! or inline, and `\{{` is a literal `{{`.

use serde_json::Value;

/// A run of one line: text, or a directive that renders inside the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Text as written, with `\{{` already turned into `{{`.
    Text(String),
    /// `{{ profile.name }}`: a value, inserted as inline markdown.
    Value(String),
    /// `{{ link /about about.md }}`: a link without the brackets markdown
    /// links show.
    Link { href: String, label: String },
}

/// A directive alone on its line, which renders whole lines, or none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// `{{ banner site.values.banner }}`: ASCII art from a value holding
    /// `art` and `alt`.
    Banner(String),
    /// `{{ collection projects where=highlight limit=3 }}`: a collection's
    /// listing rows. `filter` keeps the pages with that field set, or all of
    /// them when none has it; `limit` cuts the rows and says how many more
    /// there are.
    Collection {
        name: String,
        filter: Option<String>,
        limit: Option<usize>,
    },
    /// `{{ links }}`: the profile's links, one per row; `source` adds the
    /// `src` row.
    Links { source: bool },
    /// `{{ clone page.links.repo }}`: `git clone` for a safe URL, nothing
    /// otherwise.
    Clone(String),
}

/// What a template refers to beyond its own text, for the checks across
/// files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    /// A value inserted into a line, which must print on one line.
    Value(String),
    /// A value holding banner art.
    Banner(String),
    /// A collection, and the field it is filtered on.
    Collection {
        name: String,
        filter: Option<String>,
    },
    /// A value holding a URL, or nothing.
    Clone(String),
}

enum Directive {
    Inline(Segment),
    Block(Block),
}

/// Parses one line into text and inline directives. A block directive
/// inside a line is an error: it renders lines of its own.
pub fn segments(line: &str) -> Result<Vec<Segment>, String> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = line;
    while let Some(ch) = rest.chars().next() {
        if ch == '`'
            && let Some(end) = rest[1..].find('`')
        {
            text.push_str(&rest[..end + 2]);
            rest = &rest[end + 2..];
            continue;
        }
        if let Some(after) = rest.strip_prefix("\\{{") {
            text.push_str("{{");
            rest = after;
            continue;
        }
        if let Some(after) = rest.strip_prefix("{{") {
            let end = after.find("}}").ok_or_else(|| {
                "`{{` is never closed with `}}`; write `\\{{` for a literal".to_owned()
            })?;
            match directive(&after[..end])? {
                Directive::Inline(segment) => {
                    if !text.is_empty() {
                        out.push(Segment::Text(std::mem::take(&mut text)));
                    }
                    out.push(segment);
                }
                Directive::Block(block) => {
                    return Err(format!(
                        "`{}` renders whole lines: put it alone on its line",
                        block_name(&block)
                    ));
                }
            }
            rest = &after[end + 2..];
            continue;
        }
        text.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    if !text.is_empty() {
        out.push(Segment::Text(text));
    }
    Ok(out)
}

/// The block directive a line holds alone, if it does. A line holding only
/// an inline directive, such as a link, is an ordinary line.
pub fn block(line: &str) -> Result<Option<Block>, String> {
    let trimmed = line.trim();
    let Some(inner) = trimmed
        .strip_prefix("{{")
        .and_then(|rest| rest.strip_suffix("}}"))
    else {
        return Ok(None);
    };
    if inner.contains("{{") || inner.contains("}}") {
        return Ok(None);
    }
    match directive(inner)? {
        Directive::Block(block) => Ok(Some(block)),
        Directive::Inline(_) => Ok(None),
    }
}

/// Checks every line of a markdown source and returns what it refers to,
/// with the line number of each reference. Fenced code is skipped, as the
/// renderer shows it verbatim. Problems come back as `line N: ...`.
pub fn check_markdown(source: &str) -> Result<Vec<(usize, Reference)>, Vec<String>> {
    let mut references = Vec::new();
    let mut problems = Vec::new();
    let mut fence: Option<&str> = None;
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(&trimmed[..3]);
            continue;
        }
        let checked = match block(line) {
            Ok(Some(block)) => Ok(block_references(&block)),
            Ok(None) => segments(line).map(|parts| value_references(&parts)),
            Err(err) => Err(err),
        };
        match checked {
            Ok(found) => references.extend(found.into_iter().map(|found| (index + 1, found))),
            Err(err) => problems.push(format!("line {}: {err}", index + 1)),
        }
    }
    if problems.is_empty() {
        Ok(references)
    } else {
        Err(problems)
    }
}

/// Checks a string field that takes values but no links, such as a title,
/// and returns the values it refers to.
pub fn check_text(text: &str) -> Result<Vec<Reference>, String> {
    let parts = segments(text)?;
    if parts
        .iter()
        .any(|part| matches!(part, Segment::Link { .. }))
    {
        return Err("`link` belongs in markdown, not in this field".to_owned());
    }
    Ok(value_references(&parts))
}

/// A string field with its values filled in by `value`. A field that does
/// not parse is left as written; the checks reject it before it is stored.
pub fn expand(text: &str, value: impl Fn(&str) -> String) -> String {
    let Ok(parts) = segments(text) else {
        return text.to_owned();
    };
    parts
        .iter()
        .map(|part| match part {
            Segment::Text(text) => text.clone(),
            Segment::Value(path) => value(path),
            Segment::Link { label, .. } => label.clone(),
        })
        .collect()
}

/// The value a dotted path names inside `root`, such as `site.values.about`
/// inside `{"site": {"values": {"about": ...}}}`.
pub fn lookup<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(root, |node, key| node.get(key))
}

/// A value as a template inserts it: trimmed text for a scalar, nothing for
/// anything else.
pub fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

/// Allows http(s), mailto and same-site paths and anchors. Anything else,
/// such as `javascript:`, is rendered as text instead of a link.
pub fn safe_url(url: &str) -> Option<&str> {
    let lower = url.trim().to_ascii_lowercase();
    let ok = lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || (lower.starts_with('/') && !lower.starts_with("//"))
        || lower.starts_with('#');
    ok.then_some(url.trim())
}

fn block_references(block: &Block) -> Vec<Reference> {
    match block {
        Block::Banner(path) => vec![Reference::Banner(path.clone())],
        Block::Collection { name, filter, .. } => vec![Reference::Collection {
            name: name.clone(),
            filter: filter.clone(),
        }],
        Block::Links { .. } => Vec::new(),
        Block::Clone(path) => vec![Reference::Clone(path.clone())],
    }
}

fn value_references(parts: &[Segment]) -> Vec<Reference> {
    parts
        .iter()
        .filter_map(|part| match part {
            Segment::Value(path) => Some(Reference::Value(path.clone())),
            _ => None,
        })
        .collect()
}

fn block_name(block: &Block) -> &'static str {
    match block {
        Block::Banner(_) => "banner",
        Block::Collection { .. } => "collection",
        Block::Links { .. } => "links",
        Block::Clone(_) => "clone",
    }
}

fn directive(body: &str) -> Result<Directive, String> {
    let words = words(body)?;
    let Some((name, args)) = words.split_first() else {
        return Err("`{{ }}` is empty".to_owned());
    };
    if name.contains('.') {
        if !args.is_empty() {
            return Err(format!("the value `{name}` takes no arguments"));
        }
        value_path(name)?;
        return Ok(Directive::Inline(Segment::Value(name.clone())));
    }
    let arity = |range: std::ops::RangeInclusive<usize>, usage: &str| {
        if range.contains(&args.len()) {
            Ok(())
        } else {
            Err(format!("expected {usage}"))
        }
    };
    match name.as_str() {
        "link" => {
            arity(2..=2, "`{{ link <href> <label> }}`")?;
            if args[0].is_empty() {
                return Err("`link` needs an address".to_owned());
            }
            if safe_url(&args[0]).is_none() {
                return Err(format!(
                    "`{}` is not a link: use http(s), mailto or a path on the site",
                    args[0]
                ));
            }
            Ok(Directive::Inline(Segment::Link {
                href: args[0].clone(),
                label: args[1].clone(),
            }))
        }
        "banner" => {
            arity(1..=1, "`{{ banner <value> }}`")?;
            value_path(&args[0])?;
            Ok(Directive::Block(Block::Banner(args[0].clone())))
        }
        "clone" => {
            arity(1..=1, "`{{ clone <value> }}`")?;
            value_path(&args[0])?;
            Ok(Directive::Block(Block::Clone(args[0].clone())))
        }
        "links" => {
            arity(0..=1, "`{{ links }}` or `{{ links source }}`")?;
            match args.first().map(String::as_str) {
                None => Ok(Directive::Block(Block::Links { source: false })),
                Some("source") => Ok(Directive::Block(Block::Links { source: true })),
                Some(other) => Err(format!("`links` takes `source`, not `{other}`")),
            }
        }
        "collection" => collection(args).map(Directive::Block),
        other => Err(format!(
            "unknown directive `{other}`; there are link, banner, collection, links, clone and values such as profile.name"
        )),
    }
}

fn collection(args: &[String]) -> Result<Block, String> {
    let usage = "expected `{{ collection <name> [where=<field>] [limit=<n>] }}`";
    let Some((name, options)) = args.split_first() else {
        return Err(usage.to_owned());
    };
    if name.contains('=') || !crate::manifest::is_valid_name(name) {
        return Err(usage.to_owned());
    }
    let mut filter = None;
    let mut limit = None;
    for option in options {
        match option.split_once('=') {
            Some(("where", field)) if filter.is_none() && is_key(field) => {
                filter = Some(field.to_owned());
            }
            Some(("limit", n)) if limit.is_none() => {
                let n: usize = n
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| format!("`limit` must be a positive number, not `{n}`"))?;
                limit = Some(n);
            }
            _ => return Err(format!("`{option}`: {usage}")),
        }
    }
    Ok(Block::Collection {
        name: name.clone(),
        filter,
        limit,
    })
}

/// Checks a value path's shape. Profile fields are fixed, so they are
/// checked here; site values and page fields are checked against the
/// content that declares them.
pub fn value_path(path: &str) -> Result<(), String> {
    let keys: Vec<&str> = path.split('.').collect();
    if !keys.iter().all(|key| is_key(key)) {
        return Err(format!("`{path}` is not a value path such as profile.name"));
    }
    match keys.as_slice() {
        ["profile", "name" | "headline" | "bio" | "email"] => Ok(()),
        ["profile", "links", "github" | "linkedin" | "source"] => Ok(()),
        ["profile", "links"] => Err(
            "`profile.links` holds several links; name one, such as profile.links.github"
                .to_owned(),
        ),
        ["profile", ..] => Err(format!(
            "`{path}`: the profile has name, headline, bio, email and links.github, links.linkedin, links.source"
        )),
        ["site", "title"] => Ok(()),
        ["site", "values", _, ..] => Ok(()),
        ["site", ..] => Err(format!(
            "`{path}`: the site has title, and free values as site.values.<key>"
        )),
        ["page", _, ..] => Ok(()),
        _ => Err(format!(
            "`{path}`: values start with profile., site. or page."
        )),
    }
}

fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Splits a directive's body on whitespace, keeping `"quoted text"`, with
/// `\"` and `\\`, as one word.
fn words(body: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut chars = body.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        if chars.peek().is_none() {
            return Ok(words);
        }
        let mut word = String::new();
        while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
            if c != '"' {
                word.push(c);
                continue;
            }
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some(escaped @ ('"' | '\\')) => word.push(escaped),
                        Some(other) => {
                            word.push('\\');
                            word.push(other);
                        }
                        None => return Err("a quoted argument is never closed".to_owned()),
                    },
                    Some(c) => word.push(c),
                    None => return Err("a quoted argument is never closed".to_owned()),
                }
            }
        }
        words.push(word);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text_of(value: &str) -> Segment {
        Segment::Text(value.to_owned())
    }

    #[test]
    fn splits_values_and_links_out_of_text() {
        assert_eq!(
            segments("{{ site.values.about }} More in {{ link /about about.md }}.").unwrap(),
            [
                Segment::Value("site.values.about".to_owned()),
                text_of(" More in "),
                Segment::Link {
                    href: "/about".to_owned(),
                    label: "about.md".to_owned()
                },
                text_of("."),
            ]
        );
        assert_eq!(
            segments(r#"{{link /projects "← projects/"}}"#).unwrap(),
            [Segment::Link {
                href: "/projects".to_owned(),
                label: "← projects/".to_owned()
            }]
        );
        assert_eq!(segments("plain").unwrap(), [text_of("plain")]);
        assert!(segments("").unwrap().is_empty());
    }

    #[test]
    fn code_and_escapes_stay_literal() {
        assert_eq!(
            segments("run `{{ profile.name }}` and \\{{ this }}").unwrap(),
            [text_of("run `{{ profile.name }}` and {{ this }}")]
        );
        assert_eq!(
            segments("a lone ` then {{ page.title }}").unwrap(),
            [
                text_of("a lone ` then "),
                Segment::Value("page.title".to_owned())
            ]
        );
    }

    #[test]
    fn finds_block_directives_alone_on_a_line() {
        assert_eq!(
            block("  {{ collection projects where=highlight limit=3 }}  ").unwrap(),
            Some(Block::Collection {
                name: "projects".to_owned(),
                filter: Some("highlight".to_owned()),
                limit: Some(3)
            })
        );
        assert_eq!(
            block("{{ links source }}").unwrap(),
            Some(Block::Links { source: true })
        );
        assert_eq!(
            block("{{ banner site.values.banner }}").unwrap(),
            Some(Block::Banner("site.values.banner".to_owned()))
        );
        assert_eq!(
            block("{{ clone page.links.repo }}").unwrap(),
            Some(Block::Clone("page.links.repo".to_owned()))
        );
        assert_eq!(block("{{ link / README.md }}").unwrap(), None);
        assert_eq!(block("{{ profile.name }} and more").unwrap(), None);
        assert_eq!(block("# heading").unwrap(), None);
    }

    #[test]
    fn rejects_what_it_does_not_know() {
        for (line, expected) in [
            ("{{ nope }}", "unknown directive `nope`"),
            ("{{ profile.age }}", "the profile has name"),
            ("{{ profile.links }}", "holds several links"),
            ("{{ site.theme }}", "the site has title"),
            ("{{ values.x }}", "values start with"),
            ("x {{ links }}", "put it alone on its line"),
            ("{{ link /a }}", "expected `{{ link <href> <label> }}`"),
            ("{{ link javascript:alert(1) x }}", "is not a link"),
            ("{{ collection projects limit=0 }}", "positive number"),
            (
                "{{ collection projects sort=x }}",
                "expected `{{ collection",
            ),
            ("{{ links all }}", "takes `source`"),
            ("{{ profile.name", "never closed"),
            (r#"{{ link / "open }}"#, "never closed"),
            ("{{ }}", "is empty"),
        ] {
            let err = match block(line) {
                Err(err) => err,
                Ok(_) => segments(line).unwrap_err(),
            };
            assert!(err.contains(expected), "{line}: {err}");
        }
    }

    #[test]
    fn checks_markdown_line_by_line_outside_fences() {
        let source = "# {{ profile.name }}\n\n```\n{{ not checked }}\n```\n{{ collection blog }}\nbad {{ nope }}\n";
        let problems = check_markdown(source).unwrap_err();
        assert_eq!(
            problems,
            [
                "line 7: unknown directive `nope`; there are link, banner, collection, links, clone and values such as profile.name"
            ]
        );

        let references =
            check_markdown("{{ banner site.values.banner }}\n{{ site.values.about }}\n").unwrap();
        assert_eq!(
            references,
            [
                (1, Reference::Banner("site.values.banner".to_owned())),
                (2, Reference::Value("site.values.about".to_owned()))
            ]
        );
    }

    #[test]
    fn fields_take_values_but_not_links() {
        assert_eq!(
            check_text("{{ profile.name }}").unwrap(),
            [Reference::Value("profile.name".to_owned())]
        );
        assert!(check_text("{{ link / x }}").is_err());
    }

    #[test]
    fn expands_values_from_a_context() {
        let context = json!({"profile": {"name": " Ada "}, "site": {"values": {"n": 3}}});
        let value = |path: &str| text(lookup(&context, path));
        assert_eq!(
            expand("{{ profile.name }} · {{ site.values.n }}", value),
            "Ada · 3"
        );
        assert_eq!(expand("{{ site.values.missing }}!", value), "!");
        assert_eq!(expand("{{ broken", value), "{{ broken");
    }

    #[test]
    fn safe_urls() {
        assert_eq!(safe_url("//evil.example"), None);
        assert_eq!(safe_url("javascript:alert(1)"), None);
        assert_eq!(safe_url(" /projects "), Some("/projects"));
        assert_eq!(safe_url("mailto:a@b.c"), Some("mailto:a@b.c"));
        assert_eq!(safe_url("#top"), Some("#top"));
    }
}
