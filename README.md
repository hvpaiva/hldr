# hldr

Site and CLI for [hvpaiva.dev](https://hvpaiva.dev).

HTML in a browser. Text if you `curl`. Content is markdown in git; runtime
state lives in SQLite. Administration is a CLI in the kubectl shape — there
is no web panel. JavaScript is not required to read the pages.

The site is a sample of the craft. A merge to `main` is what turns
production.

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

`kamal deploy` from a checkout of `main`. CI runs the suite, then the same
command.

## License

[MIT](LICENSE)
