# hldr

Site and CLI for [hvpaiva.dev](https://hvpaiva.dev).

HTML in a browser. Text if you `curl`. Content is markdown in git; runtime
state lives in SQLite. Administration is a CLI in the kubectl shape — there
is no web panel. JavaScript is not required to read the pages.

The site is a sample of the craft. `fix:` and `feat:` on `main` update a
release PR ([release-plz](https://release-plz.dev)). Merging that PR
tags `v*` and deploys. A push to `main` by itself does not.

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

```
docker build -t hldr .
docker run --rm -p 8080:8080 \
  -e HLDR_ORIGIN=http://127.0.0.1:8080 \
  hldr
```

## Test

```
cargo fmt --all --check
cargo clippy --all-targets --workspace -- -D warnings
cargo test --workspace
```

## Deploy

[release-plz](https://release-plz.dev) reads conventional commits,
bumps `Cargo.toml` / `Cargo.lock`, and opens a PR. Merge it; the tag
`vMAJOR.MINOR.PATCH` is what Kamal deploys to the instance configured in
the GitHub Environment.

`workflow_dispatch` on `deploy` deploys the version already in the crate,
without bumping.

## License

[MIT](LICENSE)
