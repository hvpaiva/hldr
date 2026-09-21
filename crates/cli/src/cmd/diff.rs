use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use similar::TextDiff;

use crate::cmd::Writer;
use crate::content;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Content file, content directory, or `-` for a singleton on stdin
    #[arg(short = 'f', long = "filename", value_name = "PATH", required = true)]
    files: Vec<PathBuf>,
}

/// Prints how the branch would change if these files were applied. Like
/// `kubectl diff`, the answer is also the exit code: 0 when nothing would
/// change, 1 when something would.
pub fn run(out: &mut dyn Write, writer: &mut Writer<'_>, args: &Args) -> Result<bool> {
    let files = content::collect(&mut writer.catalog, &args.files, "diff")?;
    let head = writer.target.head()?;
    let revision = content::short(&head);
    let mut same = true;
    for file in &files {
        let current = writer.target.github.file(&head, &file.path)?;
        let before = current.as_deref().unwrap_or_default();
        if current.is_some() && before == file.text {
            continue;
        }
        same = false;
        let from = match current {
            Some(_) => format!("a/{} ({revision})", file.path),
            None => "/dev/null".to_owned(),
        };
        write!(
            out,
            "{}",
            unified(
                before,
                &file.text,
                &from,
                &format!("b/{} ({})", file.path, file.origin)
            )
        )?;
    }
    Ok(same)
}

pub fn unified(before: &str, after: &str, from: &str, to: &str) -> String {
    TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .header(from, to)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffs_are_unified() {
        let diff = unified(
            "a\nb\nc\n",
            "a\nB\nc\n",
            "a/site.yaml (abc)",
            "b/site.yaml (site.yaml)",
        );
        assert_eq!(
            diff,
            "--- a/site.yaml (abc)\n+++ b/site.yaml (site.yaml)\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"
        );
    }
}
