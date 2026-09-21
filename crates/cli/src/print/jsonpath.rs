//! kubectl-style JSONPath: `{.items[*].metadata.name}` templates and the
//! paths `custom-columns` and printer columns use.
//!
//! Expressions are RFC 9535 JSONPath with kubectl's leading `.` for the root.
//! A template mixes literal text, `{expression}` and `{"quoted text"}` with
//! `\n` and `\t` escapes. `{range}` and `{end}` are not supported.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use serde_json_path::JsonPath;

/// One expression, evaluated against a resource or a list.
#[derive(Debug)]
pub struct Path(JsonPath);

impl Path {
    pub fn parse(expression: &str) -> Result<Self> {
        let expression = expression.trim();
        let rooted = if expression.starts_with('$') {
            expression.to_owned()
        } else if expression.starts_with('.') || expression.starts_with('[') {
            format!("${expression}")
        } else {
            bail!("JSONPath must start with `.`, `[` or `$`, got {expression:?}");
        };
        JsonPath::parse(&rooted)
            .map(Self)
            .with_context(|| format!("invalid JSONPath {expression:?}"))
    }

    pub fn values<'v>(&self, value: &'v Value) -> Vec<&'v Value> {
        self.0.query(value).all()
    }
}

#[derive(Debug)]
enum Segment {
    Text(String),
    Path(Path),
}

#[derive(Debug)]
pub struct Template(Vec<Segment>);

impl Template {
    pub fn parse(template: &str) -> Result<Self> {
        let mut segments = Vec::new();
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            if open > 0 {
                segments.push(Segment::Text(rest[..open].to_owned()));
            }
            let inner = &rest[open + 1..];
            let close = closing_brace(inner)
                .with_context(|| format!("unclosed `{{` in template {template:?}"))?;
            segments.push(expression(inner[..close].trim())?);
            rest = &inner[close + 1..];
        }
        if !rest.is_empty() {
            segments.push(Segment::Text(rest.to_owned()));
        }
        Ok(Self(segments))
    }

    /// Several matches print separated by spaces, strings unquoted and other
    /// values as compact JSON, as kubectl does.
    pub fn render(&self, value: &Value) -> String {
        let mut out = String::new();
        for segment in &self.0 {
            match segment {
                Segment::Text(text) => out.push_str(text),
                Segment::Path(path) => {
                    let parts: Vec<String> = path.values(value).into_iter().map(plain).collect();
                    out.push_str(&parts.join(" "));
                }
            }
        }
        out
    }
}

fn expression(inner: &str) -> Result<Segment> {
    if let Some(quoted) = inner.strip_prefix('"') {
        let body = quoted
            .strip_suffix('"')
            .with_context(|| format!("unterminated string in {{{inner}}}"))?;
        return Ok(Segment::Text(unescape(body)?));
    }
    let keyword = inner.split_whitespace().next().unwrap_or_default();
    if matches!(keyword, "range" | "end") {
        bail!("{{{keyword}}} is not supported; use `-o json` with jq for iteration");
    }
    Ok(Segment::Path(Path::parse(inner)?))
}

/// The `}` that closes an expression, skipping those inside quotes.
fn closing_brace(inner: &str) -> Option<usize> {
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in inner.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '}' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

fn unescape(text: &str) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            other => bail!(
                "unknown escape \\{}",
                other.map(String::from).unwrap_or_default()
            ),
        }
    }
    Ok(out)
}

/// A value as a cell or template output: strings bare, the rest as JSON.
pub fn plain(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn list() -> Value {
        json!({"kind": "ProjectList", "items": [
            {"metadata": {"name": "atlas"}, "spec": {"tags": ["rust", "cubesat"], "highlight": 1}},
            {"metadata": {"name": "hldr"}, "spec": {"tags": [], "highlight": null}},
        ]})
    }

    fn render(template: &str) -> String {
        Template::parse(template).unwrap().render(&list())
    }

    #[test]
    fn renders_kubectl_templates() {
        assert_eq!(render("{.items[*].metadata.name}"), "atlas hldr");
        assert_eq!(render("{.items[0].spec.tags}"), r#"["rust","cubesat"]"#);
        assert_eq!(render("{.items[0].spec.tags[*]}"), "rust cubesat");
        assert_eq!(render(r#"{.items[0].metadata.name}{"\n"}"#), "atlas\n");
        assert_eq!(render("name={.items[1].metadata.name};"), "name=hldr;");
        assert_eq!(render("{.items[1].spec.highlight}"), "null");
        assert_eq!(render("{.items[5].metadata.name}"), "");
        assert_eq!(
            render(r#"{.items[?(@.spec.highlight==1)].metadata.name}"#),
            "atlas"
        );
        assert_eq!(render(r#"{"a}b"}"#), "a}b");
    }

    #[test]
    fn rejects_what_it_cannot_do() {
        for bad in [
            "{.items",
            "{range .items[*]}{end}",
            "{items}",
            r#"{"\q"}"#,
            "{.items[}",
        ] {
            assert!(Template::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn paths_accept_kubectl_and_rfc_roots() {
        let value = list();
        for expression in [".kind", "$.kind", "['kind']"] {
            let path = Path::parse(expression).unwrap();
            assert_eq!(path.values(&value), [&json!("ProjectList")], "{expression}");
        }
    }
}
