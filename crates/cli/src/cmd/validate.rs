//! Offline checks, with the parser and the tree checks the server indexes
//! with: no server, no config, no network.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use hldr_core::manifest::{self, Kind};
use hldr_core::tree::Tree;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Content file, or a content directory checked as a whole
    #[arg(short = 'f', long = "filename", value_name = "PATH", required = true)]
    files: Vec<PathBuf>,
}

/// Reports every problem on stderr and fails when there is one. A directory
/// is read as the server reads a revision, so the checks across files run
/// too; a file on its own goes where its declared kind and its file name say.
pub fn run(out: &mut dyn Write, args: &Args) -> Result<bool> {
    let mut valid = true;
    for arg in &args.files {
        let tree = arg.is_dir();
        let checked = if tree {
            Tree::read(arg).map(|tree| tree.files.len())
        } else {
            file(arg).map(|()| 1)
        };
        match checked {
            Ok(count) => {
                let files = if count == 1 { "file" } else { "files" };
                writeln!(out, "{}: {count} {files} valid", arg.display())?;
            }
            Err(err) => {
                valid = false;
                // A tree names its files from its root; a lone file by itself.
                let root = arg.display().to_string();
                for problem in err.problems() {
                    if tree {
                        eprintln!("error: {}/{problem}", root.trim_end_matches('/'));
                    } else {
                        eprintln!("error: {problem}");
                    }
                }
            }
        }
    }
    Ok(valid)
}

fn file(path: &Path) -> Result<(), hldr_core::Error> {
    let fail = |message: String| hldr_core::Error::file(path, message);
    let bytes = std::fs::read(path).map_err(|err| fail(err.to_string()))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| fail("not valid UTF-8".to_owned()))?;
    let declared =
        manifest::declared_kind(text).ok_or_else(|| fail("no `kind:` to say what it is".into()))?;
    let kind = Kind::from_declared(&declared).ok_or_else(|| {
        fail(format!(
            "unknown kind {declared}; this hldr knows {}",
            Kind::ALL.map(Kind::as_str).join(", ")
        ))
    })?;
    let name = path.file_stem().and_then(|stem| stem.to_str());
    let at = manifest::path(kind, if kind.is_singleton() { None } else { name });
    match manifest::parse(Path::new(&at), &bytes) {
        Ok(_) => Ok(()),
        Err(hldr_core::Error::File { message, .. }) => Err(fail(format!("as {at}: {message}"))),
        Err(other) => Err(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: &str = "kind: Site\ntitle: T\ntheme: nord\ndescriptions:\n  projects: P\n  themes: T\nblog:\n  enabled: false\n";
    const PROFILE: &str = "---\nkind: Profile\nname: N\nheadline: H\nbio: B\n---\n";
    const THEME: &str = "kind: Theme\ntitle: Nord\ndark: true\ncolors:\n  bg: \"#2e3440\"\n  fg: \"#d8dee9\"\n  fg_dim: \"#e5e9f0\"\n  fg_muted: \"#4c566a\"\n  accent: \"#88c0d0\"\n  accent2: \"#81a1c1\"\n  highlight: \"#ebcb8b\"\n  special: \"#b48ead\"\n  teal: \"#8fbcbb\"\n  error: \"#bf616a\"\n  border: \"#3b4252\"\n";

    fn checkout(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let full = dir.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        dir
    }

    fn validate(paths: &[PathBuf]) -> (bool, String) {
        let mut out = Vec::new();
        let valid = run(
            &mut out,
            &Args {
                files: paths.to_vec(),
            },
        )
        .unwrap();
        (valid, String::from_utf8(out).unwrap())
    }

    #[test]
    fn a_valid_checkout_passes() {
        let dir = checkout(&[
            ("site.yaml", SITE),
            ("profile.md", PROFILE),
            ("themes/nord.yaml", THEME),
            ("README.md", "# content\n"),
        ]);
        let (valid, out) = validate(&[dir.path().to_owned()]);
        assert!(valid);
        assert!(out.ends_with(": 3 files valid\n"), "{out}");
    }

    #[test]
    fn a_checkout_fails_the_checks_across_files() {
        let dir = checkout(&[("site.yaml", SITE), ("profile.md", PROFILE)]);
        let (valid, out) = validate(&[dir.path().to_owned()]);
        assert!(!valid);
        assert!(out.is_empty(), "{out}");
    }

    #[test]
    fn a_file_is_checked_where_it_belongs() {
        let dir = checkout(&[
            ("nord.yaml", THEME),
            ("atlas.md", "---\nkind: Project\ntitle: T\n---\n"),
            ("mystery.yaml", "kind: Widget\n"),
        ]);
        let at = |name: &str| dir.path().join(name);
        assert!(file(&at("nord.yaml")).is_ok());
        let err = file(&at("atlas.md")).unwrap_err().to_string();
        assert!(err.contains("atlas.md: as projects/atlas.md: "), "{err}");
        let err = file(&at("mystery.yaml")).unwrap_err().to_string();
        assert!(err.contains("unknown kind Widget"), "{err}");

        let (valid, out) = validate(&[at("nord.yaml"), at("atlas.md")]);
        assert!(!valid);
        assert!(out.ends_with("nord.yaml: 1 file valid\n"), "{out}");
    }
}
