//! A content tree: every file of one revision, parsed, and the checks that
//! only hold across files.
//!
//! The indexer and `hldr validate` read trees through here, so a checkout
//! that validates is one the server indexes.

use std::fs;
use std::path::{Path, PathBuf};

use crate::Error;
use crate::api::SiteSpec;
use crate::manifest::{self, Kind, Manifest};

/// A content file, parsed.
#[derive(Debug, Clone)]
pub struct File {
    /// Path under the content root.
    pub path: PathBuf,
    /// The file as stored, which the indexer hashes to skip unchanged files.
    pub bytes: Vec<u8>,
    pub manifest: Manifest,
}

/// The content files of a tree, in path order.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub files: Vec<File>,
}

impl Tree {
    /// Reads and parses every content file under `root`, then checks the
    /// tree as a whole. Every problem is reported, not only the first; the
    /// checks across files run once every file parses, so a file that failed
    /// is never also reported as missing.
    pub fn read(root: &Path) -> Result<Self, Error> {
        if !root.is_dir() {
            return Err(Error::file(root, "content directory does not exist"));
        }
        let mut files = Vec::new();
        let mut problems = Vec::new();
        for path in paths(root)? {
            match manifest::locate(&path) {
                Ok(Some(_)) => {}
                Ok(None) => continue,
                Err(err) => {
                    problems.push(err);
                    continue;
                }
            }
            let bytes = match fs::read(root.join(&path)) {
                Ok(bytes) => bytes,
                Err(err) => {
                    problems.push(Error::file(&path, err.to_string()));
                    continue;
                }
            };
            match manifest::parse(&path, &bytes) {
                Ok(manifest) => files.push(File {
                    path,
                    bytes,
                    manifest,
                }),
                Err(err) => problems.push(err),
            }
        }
        let tree = Self { files };
        if problems.is_empty() {
            problems = tree.check();
        }
        Error::collect(problems).map(|()| tree)
    }

    /// What no single file can get wrong: every singleton exists, and the
    /// site's default theme has a file.
    pub fn check(&self) -> Vec<Error> {
        let mut problems = Vec::new();
        for kind in [Kind::Site, Kind::Profile] {
            if !self.files.iter().any(|file| file.manifest.kind() == kind) {
                problems.push(Error::file(
                    manifest::path(kind, None),
                    format!("missing: every content tree has one {}", kind.as_str()),
                ));
            }
        }
        if let Some(site) = self.site()
            && !self.has(Kind::Theme, &site.theme)
        {
            problems.push(Error::file(
                manifest::path(Kind::Site, None),
                format!(
                    "theme {} has no {}",
                    site.theme,
                    manifest::path(Kind::Theme, Some(&site.theme))
                ),
            ));
        }
        problems
    }

    fn site(&self) -> Option<&SiteSpec> {
        self.files.iter().find_map(|file| match &file.manifest {
            Manifest::Site(spec) => Some(spec),
            _ => None,
        })
    }

    fn has(&self, kind: Kind, name: &str) -> bool {
        let path = PathBuf::from(manifest::path(kind, Some(name)));
        self.files.iter().any(|file| file.path == path)
    }
}

/// Files under `root`, relative to it and in order, skipping hidden entries
/// such as `.git` and `.github`.
pub fn paths(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let full = root.join(&dir);
        let entries = fs::read_dir(&full).map_err(|err| Error::file(&full, err.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|err| Error::file(&full, err.to_string()))?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = dir.join(entry.file_name());
            let kind = entry
                .file_type()
                .map_err(|err| Error::file(&path, err.to_string()))?;
            if kind.is_dir() {
                pending.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{SITE, THEME};

    const PROFILE: &str = "---\nkind: Profile\nname: N\nheadline: H\nbio: B\n---\n";
    const PROJECT: &str = "---\nkind: Project\ntitle: T\ntagline: L\nstatus: wip\n---\nBody.\n";

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let full = dir.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, text).unwrap();
        }
        dir
    }

    fn valid() -> Vec<(&'static str, &'static str)> {
        vec![
            ("site.yaml", SITE),
            ("profile.md", PROFILE),
            ("themes/nord.yaml", THEME),
            ("projects/atlas.md", PROJECT),
        ]
    }

    /// Each problem reading `files` finds, as text.
    fn problems(files: &[(&str, &str)]) -> Vec<String> {
        let dir = tree(files);
        let err = Tree::read(dir.path()).unwrap_err();
        err.problems().iter().map(ToString::to_string).collect()
    }

    #[test]
    fn reads_the_content_files_only() {
        let mut files = valid();
        files.extend([
            ("README.md", "# content\n"),
            (".github/workflows/validate.yml", "on: push\n"),
            ("projects/notes.txt", "not content\n"),
        ]);
        let dir = tree(&files);
        let read = Tree::read(dir.path()).unwrap();
        let paths: Vec<&Path> = read.files.iter().map(|f| f.path.as_path()).collect();
        assert_eq!(
            paths,
            [
                "profile.md",
                "projects/atlas.md",
                "site.yaml",
                "themes/nord.yaml"
            ]
            .map(Path::new)
        );
    }

    #[test]
    fn reports_every_file_that_fails() {
        let mut files = valid();
        files.push(("projects/bad.md", "no frontmatter"));
        files.push(("themes/odd.yaml", "kind: Site\n"));
        let err = problems(&files);
        assert_eq!(err.len(), 2, "{err:?}");
        assert!(err[0].starts_with("projects/bad.md: "), "{err:?}");
        assert!(err[1].starts_with("themes/odd.yaml: "), "{err:?}");
    }

    #[test]
    fn requires_the_singletons() {
        let err = problems(&[("themes/nord.yaml", THEME)]);
        assert_eq!(
            err,
            [
                "site.yaml: missing: every content tree has one Site",
                "profile.md: missing: every content tree has one Profile"
            ]
        );
    }

    #[test]
    fn the_default_theme_must_have_a_file() {
        let files: Vec<_> = valid()
            .into_iter()
            .filter(|(path, _)| !path.starts_with("themes/"))
            .collect();
        let err = problems(&files);
        assert_eq!(err, ["site.yaml: theme nord has no themes/nord.yaml"]);
    }

    #[test]
    fn a_file_that_fails_is_not_also_missing() {
        let mut files = valid();
        files[0] = ("site.yaml", "kind: Site\ntitle: [\n");
        let err = problems(&files);
        assert_eq!(err.len(), 1, "{err:?}");
        assert!(err[0].starts_with("site.yaml: "), "{err:?}");
    }

    #[test]
    fn a_missing_directory_is_an_error() {
        let err = Tree::read(Path::new("/nonexistent/content")).unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
    }
}
