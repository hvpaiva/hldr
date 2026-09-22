//! Offline checks, with the parser and the tree checks the server indexes
//! with: no server, no config, no network.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use hldr_core::manifest::{self, Kind, Manifest, Registry};
use hldr_core::tree::{self, Tree};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Manifest, or a content directory checked as a whole
    #[arg(short = 'f', long = "filename", value_name = "PATH", required = true)]
    files: Vec<PathBuf>,
}

/// Reports every problem on stderr and fails when there is one. A directory
/// is read as the server reads a revision, so the checks across files run
/// too. A manifest inside a checkout is checked where it sits, against the
/// page types the checkout declares; one on its own goes where its declared
/// kind and its file name say.
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
    if path.extension().is_some_and(|ext| ext == "md") {
        return Err(fail(
            "a page's markdown is checked with its manifest: give the .yaml that names it"
                .to_owned(),
        ));
    }
    let bytes = std::fs::read(path).map_err(|err| fail(err.to_string()))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| fail("not valid UTF-8".to_owned()))?;
    let declared =
        manifest::declared_kind(text).ok_or_else(|| fail("no `kind:` to say what it is".into()))?;
    let (at, registry) = match checkout(path) {
        Some((root, rel)) => (rel, tree::read_registry(&root)?),
        None => {
            let kind = Kind::from_declared(&declared).ok_or_else(|| {
                fail(format!(
                    "{declared} is no built-in kind: page types come from a checkout's kinds/, \
                     so check this file inside its checkout"
                ))
            })?;
            let name = path.file_stem().and_then(|stem| stem.to_str());
            let at = manifest::path(kind, if kind.is_singleton() { None } else { name });
            (PathBuf::from(at), Registry::default())
        }
    };
    // Problems name the file as given, and where it was checked.
    let manifest = manifest::parse(&at, &bytes, &registry).map_err(|err| {
        let mut problems: Vec<hldr_core::Error> = err
            .problems()
            .into_iter()
            .map(|problem| match problem {
                hldr_core::Error::File { message, .. } => {
                    fail(format!("as {}: {message}", at.display()))
                }
                other => fail(other.to_string()),
            })
            .collect();
        match problems.len() {
            1 => problems.remove(0),
            _ => hldr_core::Error::Many(problems),
        }
    })?;
    // The markdown it names, when it sits beside the manifest.
    if let Manifest::Page(page) = &manifest
        && let Some(file) = page.spec.content.as_ref().and_then(|c| c.file.as_ref())
    {
        let beside = path.with_file_name(file);
        let text = std::fs::read_to_string(&beside)
            .map_err(|err| fail(format!("spec.content.file {file}: {err}")))?;
        registry
            .check_content(&page.kind, &text)
            .map_err(|problems| hldr_core::Error::file(&beside, problems.join("\n")))?;
    }
    Ok(())
}

/// The checkout a file sits in, if any: the nearest directory above it with
/// a `site.yaml`, and the file's path under it when that path is part of the
/// layout.
fn checkout(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let full = path.canonicalize().ok()?;
    let root = full
        .ancestors()
        .skip(1)
        .find(|dir| dir.join("site.yaml").is_file())?;
    let rel = full.strip_prefix(root).ok()?.to_owned();
    manifest::locate(&rel).ok().flatten()?;
    Some((root.to_owned(), rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: &str = "kind: Site\nspec:\n  title: T\n  theme: nord\n";
    const PROFILE: &str = "kind: Profile\nspec:\n  name: N\n  headline: H\n  bio: B\n";
    const NAV: &str =
        "kind: Nav\nspec:\n  sections:\n    - name: files\n      items:\n        - page: index\n";
    const INDEX: &str =
        "kind: Page\nmetadata:\n  name: index\n  title: I\nspec:\n  content:\n    file: index.md\n";
    const NOT_FOUND: &str = "kind: Page\nmetadata:\n  name: not-found\n  title: \"404\"\nspec:\n  content:\n    inline: x\n";
    const KIND: &str = "kind: PageKind\nmetadata:\n  name: note\nspec:\n  names: {kind: Note, singular: note, plural: notes}\n  fields:\n    mood: {type: string, required: true}\n";
    const NOTES: &str = "kind: Collection\nmetadata:\n  name: notes\nspec:\n  kind: Note\n";
    const NOTE: &str = "kind: Note\nmetadata:\n  name: first\n  title: F\n  mood: calm\nspec:\n  content:\n    inline: \"{{ page.mood }}\"\n";
    const THEME: &str = "kind: Theme\nmetadata:\n  name: nord\nspec:\n  title: Nord\n  dark: true\n  colors:\n    bg: \"#2e3440\"\n    fg: \"#d8dee9\"\n    fg_dim: \"#e5e9f0\"\n    fg_muted: \"#4c566a\"\n    accent: \"#88c0d0\"\n    accent2: \"#81a1c1\"\n    highlight: \"#ebcb8b\"\n    special: \"#b48ead\"\n    teal: \"#8fbcbb\"\n    error: \"#bf616a\"\n    border: \"#3b4252\"\n";

    fn checkout_of(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let full = dir.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        dir
    }

    fn valid_checkout() -> tempfile::TempDir {
        checkout_of(&[
            ("site.yaml", SITE),
            ("profile.yaml", PROFILE),
            ("nav.yaml", NAV),
            ("pages/index.yaml", INDEX),
            ("pages/index.md", "# index\n"),
            ("pages/not-found.yaml", NOT_FOUND),
            ("kinds/note.yaml", KIND),
            ("collections/notes.yaml", NOTES),
            ("notes/first.yaml", NOTE),
            ("themes/nord.yaml", THEME),
            ("README.md", "# content\n"),
        ])
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
        let dir = valid_checkout();
        let (valid, out) = validate(&[dir.path().to_owned()]);
        assert!(valid, "{out}");
        assert!(out.ends_with(": 9 files valid\n"), "{out}");
    }

    #[test]
    fn a_checkout_fails_the_checks_across_files() {
        let dir = checkout_of(&[("site.yaml", SITE), ("profile.yaml", PROFILE)]);
        let (valid, out) = validate(&[dir.path().to_owned()]);
        assert!(!valid);
        assert!(out.is_empty(), "{out}");
    }

    #[test]
    fn a_file_in_a_checkout_is_checked_against_its_page_types() {
        let dir = valid_checkout();
        let at = |name: &str| dir.path().join(name);
        assert!(file(&at("notes/first.yaml")).is_ok());
        assert!(file(&at("pages/index.yaml")).is_ok());
        std::fs::write(at("notes/first.yaml"), NOTE.replace("  mood: calm\n", "")).unwrap();
        let err = file(&at("notes/first.yaml")).unwrap_err().to_string();
        assert!(
            err.contains("as notes/first.yaml: metadata.mood is missing"),
            "{err}"
        );
        std::fs::write(at("pages/index.md"), "{{ page.mood }}\n").unwrap();
        let err = file(&at("pages/index.yaml")).unwrap_err().to_string();
        assert!(err.contains("the page has no field mood"), "{err}");
        let err = file(&at("pages/index.md")).unwrap_err().to_string();
        assert!(err.contains("checked with its manifest"), "{err}");
    }

    #[test]
    fn a_file_on_its_own_is_checked_where_it_belongs() {
        let dir = checkout_of(&[
            ("nord.yaml", THEME),
            (
                "about.yaml",
                "kind: Page\nmetadata:\n  name: about\nspec: {}\n",
            ),
            ("first.yaml", NOTE),
            ("mystery.yaml", "kind: Widget\n"),
        ]);
        let at = |name: &str| dir.path().join(name);
        assert!(file(&at("nord.yaml")).is_ok());
        let err = file(&at("about.yaml")).unwrap_err().to_string();
        assert!(err.contains("about.yaml: as pages/about.yaml: "), "{err}");
        let err = file(&at("first.yaml")).unwrap_err().to_string();
        assert!(err.contains("Note is no built-in kind"), "{err}");
        let err = file(&at("mystery.yaml")).unwrap_err().to_string();
        assert!(err.contains("Widget is no built-in kind"), "{err}");

        let (valid, out) = validate(&[at("nord.yaml"), at("about.yaml")]);
        assert!(!valid);
        assert!(out.ends_with("nord.yaml: 1 file valid\n"), "{out}");
    }
}
