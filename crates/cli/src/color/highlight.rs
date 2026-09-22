//! `-o json` and `-o yaml` colored line by line, as the serializers print
//! them: keys by depth from `data.key`, values by what they read as. Only
//! escapes are added, so a colored document still parses once they are
//! stripped.

use super::{Painter, Role, value_role};

/// Two spaces an indent, as `serde_json` pretty-prints.
pub fn json(paint: Painter<'_>, text: &str) -> String {
    if paint.level.is_none() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() * 2);
    for line in text.split_inclusive('\n') {
        let (line, newline) = split_newline(line);
        let body = line.trim_start();
        let indent = &line[..line.len() - body.len()];
        out.push_str(indent);
        // The document's own braces take the first indent.
        let depth = (indent.len() / 2).saturating_sub(1);
        let value = match quoted_end(body) {
            Some(end) if body[end..].starts_with(':') => {
                out.push_str(&paint.nth(Role::DataKey, depth, &body[..end]));
                out.push(':');
                let rest = &body[end + 1..];
                let value = rest.trim_start();
                out.push_str(&rest[..rest.len() - value.len()]);
                value
            }
            _ => body,
        };
        json_value(paint, value, &mut out);
        out.push_str(newline);
    }
    out
}

fn json_value(paint: Painter<'_>, value: &str, out: &mut String) {
    let (scalar, comma) = match value.strip_suffix(',') {
        Some(scalar) => (scalar, ","),
        None => (value, ""),
    };
    if scalar.starts_with('"') {
        out.push_str(&paint.paint(Role::DataString, scalar));
    } else if scalar.starts_with(['{', '[', '}', ']']) || scalar.is_empty() {
        out.push_str(scalar);
    } else {
        out.push_str(&paint.value(scalar));
    }
    out.push_str(comma);
}

/// Block style as `serde_saphyr` prints it: `key: value`, `- item`, and
/// `|` or `>` blocks whose lines are all string.
pub fn yaml(paint: Painter<'_>, text: &str) -> String {
    if paint.level.is_none() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() * 2);
    let mut block: Option<usize> = None;
    for line in text.split_inclusive('\n') {
        let (line, newline) = split_newline(line);
        let body = line.trim_start();
        let indent = line.len() - body.len();
        if let Some(owner) = block {
            if body.is_empty() || indent > owner {
                out.push_str(&line[..indent]);
                out.push_str(&paint.paint(Role::DataString, body));
                out.push_str(newline);
                continue;
            }
            block = None;
        }
        out.push_str(&line[..indent]);
        let mut rest = body;
        let mut column = indent;
        while let Some(item) = rest.strip_prefix("- ") {
            out.push_str("- ");
            rest = item;
            column += 2;
        }
        let value = match yaml_key_end(rest) {
            Some(end) => {
                out.push_str(&paint.nth(Role::DataKey, column / 2, &rest[..end]));
                out.push(':');
                let after = &rest[end + 1..];
                let value = after.trim_start();
                out.push_str(&after[..after.len() - value.len()]);
                if value.starts_with(['|', '>']) {
                    block = Some(column);
                    out.push_str(value);
                    out.push_str(newline);
                    continue;
                }
                value
            }
            None => rest,
        };
        out.push_str(&yaml_value(paint, value));
        out.push_str(newline);
    }
    out
}

fn yaml_value(paint: Painter<'_>, value: &str) -> String {
    if value.starts_with(['"', '\'']) {
        paint.paint(Role::DataString, value)
    } else if matches!(value, "" | "-" | "[]" | "{}") {
        value.to_owned()
    } else {
        paint.paint(value_role(value), value)
    }
}

/// Where a mapping key ends: a quoted scalar or a plain one followed by
/// `:` and a space or the end of the line.
fn yaml_key_end(text: &str) -> Option<usize> {
    let end = if text.starts_with(['"', '\'']) {
        quoted_end(text)?
    } else {
        text.find(": ")
            .or_else(|| text.strip_suffix(':').map(str::len))?
    };
    let after = &text[end..];
    (after == ":" || after.starts_with(": ")).then_some(end)
}

/// The end of a string quoted with the character `text` starts with,
/// honoring JSON's backslash escapes and YAML's doubled single quote.
fn quoted_end(text: &str) -> Option<usize> {
    let quote = text.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let mut chars = text.char_indices().skip(1).peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' if quote == '"' => {
                chars.next();
            }
            '\'' if quote == '\'' && chars.peek().is_some_and(|&(_, next)| next == '\'') => {
                chars.next();
            }
            c if c == quote => return Some(i + 1),
            _ => {}
        }
    }
    None
}

fn split_newline(line: &str) -> (&str, &str) {
    match line.strip_suffix('\n') {
        Some(body) => (body, "\n"),
        None => (line, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ColorEnv, Options, Term};
    use super::*;

    fn term() -> Term {
        let options = Options {
            force: Some("basic".to_owned()),
            preset: Some("dark".to_owned()),
            ..Options::default()
        };
        Term::new(&options, &ColorEnv::default(), None, false, false).unwrap()
    }

    fn strip(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find('\x1b') {
            out.push_str(&rest[..start]);
            let end = rest[start..]
                .find('m')
                .map_or(rest.len(), |m| start + m + 1);
            rest = &rest[end..];
        }
        out.push_str(rest);
        out
    }

    #[test]
    fn json_keys_by_depth_and_values_by_type() {
        let term = term();
        let text = "{\n  \"kind\": \"Project\",\n  \"spec\": {\n    \"n\": 1,\n    \"a\\\"b\": null,\n    \"tags\": [\n      \"x\"\n    ],\n    \"ok\": true\n  }\n}\n";
        let colored = json(term.out(), text);
        assert_eq!(strip(&colored), text);
        assert!(
            colored.contains("\x1b[96m\"kind\"\x1b[0m: \x1b[93m\"Project\"\x1b[0m,"),
            "{colored}"
        );
        assert!(
            colored.contains("\x1b[36m\"n\"\x1b[0m: \x1b[35m1\x1b[0m,"),
            "{colored}"
        );
        assert!(
            colored.contains("\x1b[36m\"a\\\"b\"\x1b[0m: \x1b[90;3mnull\x1b[0m,"),
            "{colored}"
        );
        assert!(
            colored.contains("      \x1b[93m\"x\"\x1b[0m\n"),
            "{colored}"
        );
        assert!(colored.contains("\x1b[32mtrue\x1b[0m\n"), "{colored}");
        assert_eq!(json(Term::plain().out(), text), text);
    }

    #[test]
    fn yaml_keys_items_and_blocks() {
        let term = term();
        let text = "kind: ProjectList\nitems:\n- kind: Project\n  metadata:\n    name: hldr\n    url: https://x.test/a\n  spec:\n    body: |-\n      # Title\n\n      text: here\n    'it''s': 3\n    tags:\n    - rust\n    empty: []\n    draft: false\n";
        let colored = yaml(term.out(), text);
        assert_eq!(strip(&colored), text);
        assert!(
            colored.starts_with("\x1b[96mkind\x1b[0m: \x1b[93mProjectList\x1b[0m\n"),
            "{colored}"
        );
        assert!(colored.contains("- \x1b[36mkind\x1b[0m: "), "{colored}");
        assert!(
            colored.contains("\x1b[96mname\x1b[0m: \x1b[93mhldr\x1b[0m"),
            "{colored}"
        );
        assert!(
            colored.contains("\x1b[93mhttps://x.test/a\x1b[0m"),
            "{colored}"
        );
        assert!(colored.contains("\x1b[96mbody\x1b[0m: |-\n      \x1b[93m# Title\x1b[0m\n\n      \x1b[93mtext: here\x1b[0m\n"), "{colored}");
        assert!(
            colored.contains("\x1b[96m'it''s'\x1b[0m: \x1b[35m3\x1b[0m"),
            "{colored}"
        );
        assert!(colored.contains("    - \x1b[93mrust\x1b[0m\n"), "{colored}");
        assert!(colored.contains("\x1b[96mempty\x1b[0m: []\n"), "{colored}");
        assert!(colored.contains("\x1b[31mfalse\x1b[0m\n"), "{colored}");
    }
}
