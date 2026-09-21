//! `kubectl edit`'s loop: open the file, validate what comes back, and on an
//! error reopen it with the error written into it.
//!
//! Errors go in as `# hldr:` comment lines: YAML comments at the top of a
//! YAML file, inside the frontmatter of a markdown one, so the file stays
//! readable in the editor. Those lines are removed before validating.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

const NOTE: &str = "# hldr: ";

/// Returns the edited text, or `None` when the user left it as it was.
pub fn edit(
    editor: &[String],
    file_name: &str,
    original: &str,
    validate: impl Fn(&str) -> Result<()>,
) -> Result<Option<String>> {
    let (program, args) = editor.split_first().context("no editor configured")?;
    let dir = tempfile::Builder::new().prefix("hldr-edit-").tempdir()?;
    let file = dir.path().join(file_name);
    let mut shown = original.to_owned();
    let mut failed = false;
    loop {
        std::fs::write(&file, &shown).with_context(|| format!("writing {}", file.display()))?;
        let status = Command::new(program)
            .args(args)
            .arg(&file)
            .status()
            .with_context(|| format!("running the editor ({program})"))?;
        if !status.success() {
            bail!("the editor ({program}) exited with {status}; nothing was changed");
        }
        let saved = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        let text = strip_notes(&saved);
        if text == original {
            eprintln!("edit cancelled, no changes made");
            return Ok(None);
        }
        if failed && saved == shown {
            let kept = dir.keep().join(file_name);
            bail!(
                "edit aborted with the error unfixed; your version is at {}",
                kept.display()
            );
        }
        match validate(&text) {
            Ok(()) => return Ok(Some(text)),
            Err(err) => {
                shown = annotate(&text, &format!("{err:#}"));
                failed = true;
            }
        }
    }
}

/// `HLDR_EDITOR`, `VISUAL`, `EDITOR`, else `vi`; split on whitespace so
/// `code --wait` works, as kubectl does with `KUBE_EDITOR`.
pub fn command(configured: Option<&str>) -> Vec<String> {
    configured
        .unwrap_or("vi")
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

fn annotate(text: &str, error: &str) -> String {
    let mut notes =
        format!("{NOTE}this did not validate. Fix it and save, or quit without saving to abort.\n");
    for line in error.lines() {
        notes.push_str(&format!("{NOTE}  {line}\n"));
    }
    match text.strip_prefix("---\n") {
        Some(rest) => format!("---\n{notes}{rest}"),
        None => format!("{notes}{text}"),
    }
}

fn strip_notes(text: &str) -> String {
    text.split_inclusive('\n')
        .filter(|line| !line.starts_with(NOTE))
        .collect()
}

/// The file name to edit under, so the editor picks the right syntax.
pub fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(
        || "content".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An "editor" that runs a shell script on the file, then counts runs.
    fn script(body: &str) -> Vec<String> {
        vec![
            "sh".to_owned(),
            "-c".to_owned(),
            body.to_owned(),
            "editor".to_owned(),
        ]
    }

    fn valid(text: &str) -> Result<()> {
        if text.contains("bad") {
            bail!("line 2: bad value\nsecond line of detail");
        }
        Ok(())
    }

    #[test]
    fn returns_what_the_editor_saved() {
        let edited = edit(
            &script(r#"sed -i 's/old/new/' "$1""#),
            "a.md",
            "---\nx: old\n---\n",
            valid,
        )
        .unwrap();
        assert_eq!(edited.as_deref(), Some("---\nx: new\n---\n"));
    }

    #[test]
    fn leaving_it_unchanged_cancels() {
        assert_eq!(
            edit(&script("true"), "a.md", "---\nx: 1\n---\n", valid).unwrap(),
            None
        );
    }

    #[test]
    fn reopens_with_the_error_until_fixed() {
        // First run writes an invalid value; the second sees the notes and fixes it.
        let body = r#"if grep -q '^# hldr: ' "$1"; then sed -i 's/bad/good/' "$1"; else sed -i 's/x: 1/x: bad/' "$1"; fi"#;
        let edited = edit(&script(body), "a.md", "---\nx: 1\n---\nbody\n", valid).unwrap();
        assert_eq!(edited.as_deref(), Some("---\nx: good\n---\nbody\n"));
    }

    #[test]
    fn saving_the_error_unchanged_aborts_and_keeps_the_file() {
        let body = r#"grep -q '^# hldr: ' "$1" || sed -i 's/x: 1/x: bad/' "$1""#;
        let err = edit(&script(body), "a.md", "---\nx: 1\n---\n", valid).unwrap_err();
        let message = err.to_string();
        let kept = message.rsplit(' ').next().unwrap();
        assert!(
            std::fs::read_to_string(kept).unwrap().contains("x: bad"),
            "{message}"
        );
        std::fs::remove_dir_all(Path::new(kept).parent().unwrap()).unwrap();
    }

    #[test]
    fn notes_sit_inside_the_frontmatter() {
        let annotated = annotate("---\nx: bad\n---\nbody\n", "oops\nmore");
        assert_eq!(
            annotated,
            "---\n# hldr: this did not validate. Fix it and save, or quit without saving to abort.\n\
             # hldr:   oops\n# hldr:   more\nx: bad\n---\nbody\n"
        );
        assert_eq!(strip_notes(&annotated), "---\nx: bad\n---\nbody\n");
        assert!(annotate("kind: Site\n", "e").starts_with("# hldr: "));
    }

    #[test]
    fn editor_commands_split_like_kubectl() {
        assert_eq!(command(None), ["vi"]);
        assert_eq!(command(Some("code --wait")), ["code", "--wait"]);
    }
}
