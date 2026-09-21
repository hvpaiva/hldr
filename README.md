# hldr

Site and CLI for [hvpaiva.dev](https://hvpaiva.dev).

HTML in a browser. Text if you `curl`. Content is markdown in git; runtime
state lives in SQLite. Administration is a CLI in the kubectl shape — there
is no web panel. JavaScript is not required to read the pages.

The site is a sample of the craft. Production moves on a `v*` tag, or
Actions → Run workflow → `bump: patch|minor|major`. The job writes
`Cargo.toml` and `Cargo.lock` from that version; do not edit them by
hand. A push to `main` runs the suite and does not deploy.

## Workspace

```
crates/core       domain, SQLite, markdown
crates/server     hldr-server
crates/cli        hldr
content/          desired state (profile, projects)
migrations/       sqlx
config/deploy.yml Kamal
```

## Run

```
cargo build --workspace
HLDR_CONTENT_DIR=content cargo run -p hldr-server
```

Listens on `127.0.0.1:8080`. `HLDR_ADDR` overrides. `/healthz` does not
touch the database; `/readyz` does. SQLite defaults to `./hldr.db`.

```
cargo run -p hldr -- version
```

## Test

```
cargo fmt --all --check
cargo clippy --all-targets --workspace -- -D warnings
cargo test --workspace
```

## Deploy

Tag `vMAJOR.MINOR.PATCH` and push it, or run the `ci` workflow with
`bump`. Either way CI sets the crate version (and the lockfile) before
`kamal deploy --version <semver>`.

```
python3 scripts/version.py bump patch   # local; then commit and tag
```

## License

[MIT](LICENSE)
