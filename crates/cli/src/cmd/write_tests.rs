//! The writing commands end to end, against an in-memory GitHub repository
//! and the server's sync endpoint.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::Parser;

use crate::client::Client;
use crate::cmd::Writer;
use crate::color::Term;
use crate::discovery::Catalog;
use crate::stub::hub::{self, Repo, Shared};

const SITE: &str = "kind: Site\nspec:\n  title: example.test\n  theme: nord\n";
const ATLAS: &str = "kind: Project\nmetadata:\n  name: atlas\n  title: Atlas\n  tagline: Old line\n  status: active\nspec:\n  content:\n    file: atlas.md\n";
const ATLAS_MD: &str = "Body.\n";

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
                ("projects/atlas.yaml", ATLAS),
                ("projects/atlas.md", ATLAS_MD),
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
    run_with(world, token, None, args)
}

fn run_with(
    world: &World,
    token: Option<&'static str>,
    editor: Option<String>,
    args: &[&str],
) -> (Result<bool>, String) {
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
        Edit(super::edit::Args),
    }
    let parsed = Test::try_parse_from(std::iter::once("hldr").chain(args.iter().copied())).unwrap();
    let mut out = Vec::new();
    let mut writer = world.writer(token, editor);
    let term = Term::plain();
    let result = match &parsed.command {
        Command::Apply(a) => super::apply::run(&mut out, &term, &mut writer, a),
        Command::Diff(a) => super::diff::run(&mut out, &term, &mut writer, a),
        Command::Patch(a) => super::patch::run(&mut out, &term, &mut writer, a),
        Command::Delete(a) => super::delete::run(&mut out, &term, &mut writer, a),
        Command::Edit(a) => super::edit::run(&mut out, &mut writer, a),
    };
    (result, String::from_utf8(out).unwrap())
}

#[test]
fn apply_commits_only_what_changed_and_syncs_that_commit() {
    let world = World::new();
    let atlas = world.file("atlas.yaml", &ATLAS.replace("Old line", "New line"));
    world.file("atlas.md", ATLAS_MD);
    let hermes = world.file(
        "hermes.yaml",
        &ATLAS.replace("atlas", "hermes").replace("Atlas", "Hermes"),
    );
    world.file("hermes.md", "Hermes.\n");
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
    assert!(files["projects/atlas.yaml"].contains("New line"));
    assert_eq!(files["projects/atlas.md"], ATLAS_MD);
    assert!(files["projects/hermes.yaml"].contains("Hermes"));
    assert_eq!(
        files["projects/hermes.md"], "Hermes.\n",
        "the markdown comes along"
    );
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
fn a_changed_markdown_alone_configures_its_page() {
    let world = World::new();
    let atlas = world.file("atlas.yaml", ATLAS);
    world.file("atlas.md", "Rewritten.\n");
    let (result, out) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", atlas.to_str().unwrap()],
    );
    assert!(result.unwrap(), "{out}");
    assert!(out.starts_with("project/atlas configured\n"), "{out}");
    assert_eq!(world.head_files()["projects/atlas.md"], "Rewritten.\n");
}

#[test]
fn a_manifest_needs_its_markdown_beside_it_or_on_the_branch() {
    let world = World::new();
    let atlas = world.file("alone/atlas.yaml", &ATLAS.replace("Old line", "New"));
    let (result, out) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", atlas.to_str().unwrap()],
    );
    assert!(result.unwrap(), "the branch has projects/atlas.md: {out}");

    let hermes = world.file(
        "alone/hermes.yaml",
        &ATLAS.replace("atlas", "hermes").replace("Atlas", "Hermes"),
    );
    let (result, _) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", hermes.to_str().unwrap()],
    );
    let err = format!("{:#}", result.unwrap_err());
    assert!(
        err.contains(
            "names projects/hermes.md as its content, which is neither beside it nor on main"
        ),
        "{err}"
    );
}

#[test]
fn apply_reads_a_checkout_and_skips_what_is_not_content() {
    let world = World::new();
    world.file("tree/README.md", "not content\n");
    world.file("tree/site.yaml", SITE);
    world.file(
        "tree/projects/atlas.yaml",
        &ATLAS.replace("Old line", "From tree"),
    );
    world.file("tree/projects/atlas.md", "From tree.\n");
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
    let files = world.head_files();
    assert!(files["projects/atlas.yaml"].contains("From tree"));
    assert_eq!(files["projects/atlas.md"], "From tree.\n");
    assert!(world.repo.lock().unwrap().synced.is_empty());

    world.file("tree/projects/stray.md", "nobody names me\n");
    let (result, _) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", tree.to_str().unwrap()],
    );
    assert!(format!("{:#}", result.unwrap_err()).contains("no page names it"));
}

#[test]
fn dry_runs_and_invalid_files_commit_nothing() {
    let world = World::new();
    let before = world.repo.lock().unwrap().commit_count();
    let changed = world.file("atlas.yaml", &ATLAS.replace("Old line", "New"));
    let (result, out) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", changed.to_str().unwrap(), "--dry-run"],
    );
    assert!(result.unwrap());
    assert_eq!(out, "project/atlas configured (dry run)\n");

    let broken = world.file(
        "broken.yaml",
        &ATLAS
            .replace("name: atlas", "name: broken")
            .replace("status: active", "status: done"),
    );
    let (result, _) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", broken.to_str().unwrap()],
    );
    let err = format!("{:#}", result.unwrap_err());
    assert!(
        err.contains("broken.yaml") && err.contains("must be one of"),
        "{err}"
    );

    let markdown = world.file("atlas.md", "{{ nope }}\n");
    let (result, _) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", changed.to_str().unwrap()],
    );
    let err = format!("{:#}", result.unwrap_err());
    assert!(
        err.contains("atlas.md") && err.contains("unknown directive"),
        "{err}"
    );
    let (result, _) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", markdown.to_str().unwrap()],
    );
    assert!(format!("{:#}", result.unwrap_err()).contains("goes with its manifest"));
    assert_eq!(world.repo.lock().unwrap().commit_count(), before);
}

#[test]
fn apply_leaves_out_what_the_server_keeps() {
    let world = World::new();
    let printed = world.file(
        "atlas.yaml",
        &format!(
            "{}status: {{}}\n",
            ATLAS
                .replace("Old line", "From get")
                .replace("  status: active\n", "  status: active\n  updated_at: t\n")
        ),
    );
    let (result, out) = run(
        &world,
        Some("t0ken"),
        &["apply", "-f", printed.to_str().unwrap()],
    );
    assert!(result.unwrap(), "{out}");
    let atlas = &world.head_files()["projects/atlas.yaml"];
    assert!(
        atlas.contains("From get") && !atlas.contains("updated_at"),
        "{atlas}"
    );
}

#[test]
fn writes_need_a_token() {
    let world = World::new();
    let changed = world.file("atlas.yaml", &ATLAS.replace("Old line", "New"));
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
        .push(&[("projects/atlas.yaml", "pushed elsewhere\n")], "other");
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
        world.head_files()["projects/atlas.yaml"],
        "pushed elsewhere\n"
    );
}

#[test]
fn diff_answers_with_its_result() {
    let world = World::new();
    let same = world.file("atlas.yaml", ATLAS);
    world.file("atlas.md", ATLAS_MD);
    let (result, out) = run(&world, None, &["diff", "-f", same.to_str().unwrap()]);
    assert!(result.unwrap());
    assert!(out.is_empty(), "{out}");

    let changed = world.file("atlas.yaml", &ATLAS.replace("Old line", "New line"));
    world.file("atlas.md", "Body, changed.\n");
    let (result, out) = run(&world, None, &["diff", "-f", changed.to_str().unwrap()]);
    assert!(!result.unwrap());
    assert!(
        out.contains("-  tagline: Old line\n+  tagline: New line\n"),
        "{out}"
    );
    assert!(out.starts_with("--- a/projects/atlas.yaml ("), "{out}");
    assert!(
        out.contains("--- a/projects/atlas.md (") && out.contains("+Body, changed.\n"),
        "{out}"
    );
}

#[test]
fn patch_rewrites_the_manifest_from_its_fields() {
    let world = World::new();
    let (result, out) = run(
        &world,
        Some("t0ken"),
        &[
            "patch",
            "project",
            "atlas",
            "-p",
            r#"{"metadata":{"tagline":"Patched","tags":["rust"]}}"#,
        ],
    );
    assert!(result.unwrap(), "{out}");
    let atlas = &world.head_files()["projects/atlas.yaml"];
    assert!(atlas.contains("tagline: Patched"), "{atlas}");
    assert!(atlas.contains("file: atlas.md"), "{atlas}");

    let (result, out) = run(
        &world,
        Some("t0ken"),
        &[
            "patch",
            "site",
            "-p",
            r#"{"spec":{"title":"changed.test"}}"#,
        ],
    );
    assert!(result.unwrap(), "{out}");
    assert!(out.starts_with("site patched\n"), "{out}");
    assert!(world.head_files()["site.yaml"].contains("title: changed.test"));

    let before = world.repo.lock().unwrap().commit_count();
    let again = r#"{"spec":{"title":"changed.test"}}"#;
    let (result, out) = run(&world, Some("t0ken"), &["patch", "site", "-p", again]);
    assert!(result.unwrap());
    assert_eq!(out, "site patched (no change)\n");
    assert_eq!(world.repo.lock().unwrap().commit_count(), before);

    let unknown = r#"{"metadata":{"tagln":"x"}}"#;
    let (result, _) = run(
        &world,
        Some("t0ken"),
        &["patch", "project", "atlas", "-p", unknown],
    );
    let err = format!("{:#}", result.unwrap_err());
    assert!(
        err.contains("metadata.tagln is not a field of Project"),
        "{err}"
    );
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
    let files = world.head_files();
    assert!(!files.contains_key("projects/atlas.yaml"));
    assert!(
        !files.contains_key("projects/atlas.md"),
        "its markdown goes too"
    );

    let (result, _) = run(&world, Some("t0ken"), &["delete", "site", "x"]);
    assert!(result.is_err());
}

fn editor(world: &World, script: &str) -> String {
    let editor = world.file("editor.sh", &format!("#!/bin/sh\n{script}\n"));
    std::fs::set_permissions(&editor, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    editor.display().to_string()
}

#[test]
fn edit_commits_what_the_editor_saved() {
    let world = World::new();
    let edit = editor(&world, "sed -i 's/Old line/Edited/' \"$1\"");
    let (result, out) = run_with(
        &world,
        Some("t0ken"),
        Some(edit),
        &["edit", "project", "atlas"],
    );
    assert!(result.unwrap(), "{out}");
    assert!(world.head_files()["projects/atlas.yaml"].contains("tagline: Edited"));
    let repo = world.repo.lock().unwrap();
    assert!(
        repo.message(&repo.head)
            .starts_with("content: edit project/atlas")
    );
}

#[test]
fn edit_content_opens_the_markdown_the_manifest_names() {
    let world = World::new();
    let edit = editor(&world, "sed -i 's/Body/Edited body/' \"$1\"");
    let (result, out) = run_with(
        &world,
        Some("t0ken"),
        Some(edit),
        &["edit", "project", "atlas", "--content"],
    );
    assert!(result.unwrap(), "{out}");
    let files = world.head_files();
    assert_eq!(files["projects/atlas.md"], "Edited body.\n");
    assert_eq!(
        files["projects/atlas.yaml"], ATLAS,
        "the manifest is left alone"
    );
}
