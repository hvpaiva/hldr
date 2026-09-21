//! The writing commands end to end, against an in-memory GitHub repository
//! and the server's sync endpoint.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::Parser;

use crate::client::Client;
use crate::cmd::Writer;
use crate::discovery::Catalog;
use crate::stub::hub::{self, Repo, Shared};

const SITE: &str = "kind: Site\ntitle: example.test\ntheme: nord\ndescriptions:\n  projects: P.\n  themes: T.\nblog:\n  enabled: false\n";
const ATLAS: &str =
    "---\nkind: Project\ntitle: Atlas\ntagline: Old line\nstatus: active\n---\n\nBody.\n";

struct World {
    repo: Shared,
    client: Client,
    base: String,
    dir: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        let repo: Shared = Arc::new(Mutex::new(Repo::default()));
        repo.lock().unwrap().push(
            &[
                ("site.yaml", SITE),
                ("projects/atlas.md", ATLAS),
                ("README.md", "hi\n"),
            ],
            "init",
        );
        let base = hub::serve(Arc::clone(&repo));
        Self {
            repo,
            client: Client::new(base.clone()),
            base,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn writer(&self, token: Option<&'static str>, editor: Option<String>) -> Writer<'_> {
        Writer::new(
            &self.client,
            Catalog::new(&self.client, None),
            &self.base,
            editor,
            token.map(str::to_owned),
        )
        .unwrap()
    }

    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, text).unwrap();
        path
    }

    fn head_files(&self) -> std::collections::BTreeMap<String, String> {
        let repo = self.repo.lock().unwrap();
        repo.files(&repo.head)
    }
}

/// Parses `args` as clap would and runs the command against the world.
fn run(world: &World, token: Option<&'static str>, args: &[&str]) -> (Result<bool>, String) {
    #[derive(Parser)]
    struct Test {
        #[command(subcommand)]
        command: Command,
    }
    #[derive(clap::Subcommand)]
    enum Command {
        Apply(super::apply::Args),
        Diff(super::diff::Args),
        Patch(super::patch::Args),
        Delete(super::delete::Args),
    }
    let parsed = Test::try_parse_from(std::iter::once("hldr").chain(args.iter().copied())).unwrap();
    let mut out = Vec::new();
    let mut writer = world.writer(token, None);
    let result = match &parsed.command {
        Command::Apply(a) => super::apply::run(&mut out, &mut writer, a),
        Command::Diff(a) => super::diff::run(&mut out, &mut writer, a),
        Command::Patch(a) => super::patch::run(&mut out, &mut writer, a),
        Command::Delete(a) => super::delete::run(&mut out, &mut writer, a),
    };
    (result, String::from_utf8(out).unwrap())
}

#[test]
fn apply_commits_only_what_changed_and_syncs_that_commit() {
    let world = World::new();
    let atlas = world.file("atlas.md", &ATLAS.replace("Old line", "New line"));
    let hermes = world.file("hermes.md", &ATLAS.replace("Atlas", "Hermes"));
    let site = world.file("site.yaml", SITE);
    let (result, out) = run(
        &world,
        Some("t0ken"),
        &[
            "apply",
            "-f",
            atlas.to_str().unwrap(),
            "-f",
            hermes.to_str().unwrap(),
            "-f",
            site.to_str().unwrap(),
        ],
    );
    assert!(result.unwrap(), "{out}");
    assert!(
        out.starts_with(
            "project/atlas configured\nproject/hermes created\nsite unchanged\ncommitted "
        ),
        "{out}"
    );

    let files = world.head_files();
    assert!(files["projects/atlas.md"].contains("New line"));
    assert!(files["projects/hermes.md"].contains("Hermes"));
    let repo = world.repo.lock().unwrap();
    assert_eq!(repo.synced, [Some(repo.head.clone())]);
    let synced = format!("synced: {}, updated 1 project\n", &repo.head[..12]);
    assert!(out.ends_with(&synced), "{out}");
    let message = repo.message(&repo.head);
    assert!(
        message.starts_with("content: apply project/atlas, project/hermes\n\nHldr-Client: hldr "),
        "{message}"
    );
}

#[test]
fn apply_reads_a_checkout_and_skips_what_is_not_content() {
    let world = World::new();
    world.file("tree/README.md", "not content\n");
    world.file("tree/site.yaml", SITE);
    world.file(
        "tree/projects/atlas.md",
        &ATLAS.replace("Old line", "From tree"),
    );
    let tree = world.dir.path().join("tree");
    let (result, out) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", tree.to_str().unwrap(), "--no-sync"],
    );
    assert!(result.unwrap(), "{out}");
    assert!(
        out.contains("project/atlas configured\nsite unchanged\n"),
        "{out}"
    );
    assert!(out.contains("not synced"), "{out}");
    assert!(world.head_files()["projects/atlas.md"].contains("From tree"));
    assert!(world.repo.lock().unwrap().synced.is_empty());
}

#[test]
fn dry_runs_and_invalid_files_commit_nothing() {
    let world = World::new();
    let before = world.repo.lock().unwrap().commit_count();
    let changed = world.file("atlas.md", &ATLAS.replace("Old line", "New"));
    let (result, out) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", changed.to_str().unwrap(), "--dry-run"],
    );
    assert!(result.unwrap());
    assert_eq!(out, "project/atlas configured (dry run)\n");

    let broken = world.file(
        "broken.md",
        &ATLAS.replace("status: active", "status: done"),
    );
    let (result, _) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", broken.to_str().unwrap()],
    );
    assert!(format!("{:#}", result.unwrap_err()).contains("broken.md"));
    assert_eq!(world.repo.lock().unwrap().commit_count(), before);
}

#[test]
fn writes_need_a_token() {
    let world = World::new();
    let changed = world.file("atlas.md", &ATLAS.replace("Old line", "New"));
    let (result, _) = run(&world, None, &["apply", "-f", changed.to_str().unwrap()]);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("writing needs a GitHub token")
    );
}

#[test]
fn a_branch_that_moved_is_not_overwritten() {
    let world = World::new();
    let target = world.writer(Some("t0ken"), None).target;
    let stale = target.head().unwrap();
    world
        .repo
        .lock()
        .unwrap()
        .push(&[("projects/atlas.md", "pushed elsewhere\n")], "other");
    let github = target.github;
    let change = crate::github::Change {
        path: "site.yaml".to_owned(),
        content: Some(SITE.to_owned()),
    };
    let err = github.commit("main", &stale, &[change], "m").unwrap_err();
    assert!(
        err.to_string()
            .contains("main moved while this change was being written"),
        "{err}"
    );
    assert_eq!(
        world.head_files()["projects/atlas.md"],
        "pushed elsewhere\n"
    );
}

#[test]
fn diff_answers_with_its_result() {
    let world = World::new();
    let same = world.file("atlas.md", ATLAS);
    let (result, out) = run(&world, None, &["diff", "-f", same.to_str().unwrap()]);
    assert!(result.unwrap());
    assert!(out.is_empty());

    let changed = world.file("atlas.md", &ATLAS.replace("Old line", "New line"));
    let (result, out) = run(&world, None, &["diff", "-f", changed.to_str().unwrap()]);
    assert!(!result.unwrap());
    assert!(
        out.contains("-tagline: Old line\n+tagline: New line\n"),
        "{out}"
    );
    assert!(out.starts_with("--- a/projects/atlas.md ("), "{out}");
}

#[test]
fn patch_rewrites_the_file_from_its_fields() {
    let world = World::new();
    let (result, out) = run(
        &world,
        Some("t0ken"),
        &[
            "patch",
            "project",
            "atlas",
            "-p",
            r#"{"spec":{"tagline":"Patched","tags":["rust"]}}"#,
        ],
    );
    assert!(result.unwrap(), "{out}");
    let atlas = &world.head_files()["projects/atlas.md"];
    assert!(atlas.contains("tagline: Patched"), "{atlas}");
    assert!(atlas.ends_with("---\n\nBody.\n"), "{atlas}");

    let (result, out) = run(
        &world,
        Some("t0ken"),
        &[
            "patch",
            "site",
            "-p",
            r#"{"spec":{"blog":{"enabled":true}}}"#,
        ],
    );
    assert!(result.unwrap(), "{out}");
    assert!(out.starts_with("site patched\n"), "{out}");
    assert!(world.head_files()["site.yaml"].contains("enabled: true"));

    let before = world.repo.lock().unwrap().commit_count();
    let again = r#"{"spec":{"blog":{"enabled":true}}}"#;
    let (result, out) = run(&world, Some("t0ken"), &["patch", "site", "-p", again]);
    assert!(result.unwrap());
    assert_eq!(out, "site patched (no change)\n");
    assert_eq!(world.repo.lock().unwrap().commit_count(), before);
}

#[test]
fn delete_removes_what_exists_and_reports_the_rest() {
    let world = World::new();
    let (result, out) = run(
        &world,
        Some("t0ken"),
        &["delete", "project", "atlas", "ghost"],
    );
    assert!(!result.unwrap(), "a missing name fails the command");
    assert!(out.contains("project/atlas deleted"), "{out}");
    assert!(!world.head_files().contains_key("projects/atlas.md"));

    let (result, _) = run(&world, Some("t0ken"), &["delete", "site", "x"]);
    assert!(result.is_err());
}

#[test]
fn edit_commits_what_the_editor_saved() {
    let world = World::new();
    let editor = world.file(
        "editor.sh",
        "#!/bin/sh\nsed -i 's/Old line/Edited/' \"$1\"\n",
    );
    std::fs::set_permissions(&editor, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut out = Vec::new();
    let mut writer = world.writer(Some("t0ken"), Some(editor.display().to_string()));
    let args = <super::edit::Args as clap::FromArgMatches>::from_arg_matches(
        &<super::edit::Args as clap::Args>::augment_args(clap::Command::new("edit"))
            .get_matches_from(["edit", "project", "atlas"]),
    )
    .unwrap();
    assert!(super::edit::run(&mut out, &mut writer, &args).unwrap());
    assert!(world.head_files()["projects/atlas.md"].contains("tagline: Edited"));
    let repo = world.repo.lock().unwrap();
    assert!(
        repo.message(&repo.head)
            .starts_with("content: edit project/atlas")
    );
}
