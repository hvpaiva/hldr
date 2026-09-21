use comrak::nodes::{AstNode, NodeValue};
use comrak::options::Plugins;
use comrak::plugins::syntect::SyntectAdapterBuilder;
use comrak::{Arena, Options, parse_document};

pub struct Markdown {
    adapter: comrak::plugins::syntect::SyntectAdapter,
}

impl Markdown {
    pub fn new() -> Self {
        Self {
            adapter: SyntectAdapterBuilder::new().css().build(),
        }
    }

    pub fn html(&self, source: &str) -> String {
        let mut plugins = Plugins::default();
        plugins.render.codefence_syntax_highlighter = Some(&self.adapter);
        comrak::markdown_to_html_with_plugins(source, &gfm_options(), &plugins)
    }

    pub fn text(source: &str) -> String {
        let arena = Arena::new();
        let root = parse_document(&arena, source, &gfm_options());
        let mut out = String::new();
        collect_text(root, &mut out);
        collapse_blank_lines(&out)
    }
}

impl Default for Markdown {
    fn default() -> Self {
        Self::new()
    }
}

fn gfm_options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.tagfilter = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.footnotes = true;
    options
}

fn collect_text<'a>(node: &'a AstNode<'a>, out: &mut String) {
    match &node.data.borrow().value {
        NodeValue::Text(text) => {
            out.push_str(text);
            return;
        }
        NodeValue::Code(code) => {
            out.push_str(&code.literal);
            return;
        }
        NodeValue::CodeBlock(block) => {
            push_newline(out);
            out.push_str(&block.literal);
            push_newline(out);
            return;
        }
        NodeValue::SoftBreak | NodeValue::LineBreak => {
            out.push('\n');
            return;
        }
        _ => {}
    }

    for child in node.children() {
        collect_text(child, out);
    }

    if matches!(
        node.data.borrow().value,
        NodeValue::Paragraph | NodeValue::Heading(_) | NodeValue::Item(_)
    ) {
        push_newline(out);
        out.push('\n');
    }
}

fn push_newline(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn collapse_blank_lines(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut blank = false;
    for line in input.lines() {
        if line.trim().is_empty() {
            if !out.is_empty() && !blank {
                out.push('\n');
                blank = true;
            }
            continue;
        }
        out.push_str(line.trim_end());
        out.push('\n');
        blank = false;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::Markdown;

    #[test]
    fn html_highlights_fenced_rust() {
        let md = Markdown::new();
        let html = md.html("```rust\nfn main() {}\n```\n");
        assert!(html.contains("<pre"), "{html}");
        assert!(html.contains("class="), "{html}");
    }

    #[test]
    fn text_keeps_paragraph_and_code() {
        let text = Markdown::text("# Title\n\nHello *world*.\n\n```\ncode\n```\n");
        assert!(text.contains("Title"), "{text}");
        assert!(text.contains("Hello world."), "{text}");
        assert!(text.contains("code"), "{text}");
    }
}
