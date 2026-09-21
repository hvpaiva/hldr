# hldr

Site and CLI for [hvpaiva.dev](https://hvpaiva.dev).

HTML in a browser. Content is markdown in git; runtime state lives in
SQLite. Administration is a CLI in the kubectl shape; there
is no web panel. JavaScript is not required to read the pages.

The site is a sample of the craft. A push to `main` that changes the site
is a release: checked, built, deployed, and tagged `vX.Y.Z` once
production is healthy. The tag is the version; nothing is committed back.

## Workspace

```
crates/core       domain, SQLite, markdown
crates/server     hldr-server
crates/cli        hldr
content/          desired state (profile, projects)
migrations/       sqlx
config/           Kamal: deploy.yml, the target's pinned host keys
cliff.toml        version bumps and release notes (git-cliff)
```

## Run

```
cargo build --workspace
HLDR_CONTENT_DIR=content cargo run -p hldr-server
```

Listens on `127.0.0.1:8080`. `HLDR_ADDR` overrides. `/healthz` does not
touch the database; `/readyz` does. Both report `version` and `revision`.
SQLite defaults to `./hldr.db`. `HLDR_ORIGIN` sets the canonical URL and
defaults to the local listener.

```
cargo run -p hldr -- version
```

```
docker build -t hldr .
docker run --rm -p 8080:8080 hldr
```

Outside the release pipeline the version is `dev`. Cargo manifests carry
no version; `HLDR_VERSION` and `HLDR_REVISION` stamp it at build time.

## Test

```
cargo fmt --all --check
cargo clippy --all-targets --workspace -- -D warnings
cargo test --workspace
```

## Deploy

`.github/workflows/deploy.yml`, on every push to `main`:

1. `plan` compares the deployable inputs (`crates/`, `content/`,
   `migrations/`, Cargo files, `Dockerfile`, `config/`, Kamal gems) with
   the last `vX.Y.Z` tag. Nothing changed: nothing runs.
2. The bump comes from Conventional Commits: `fix` patch, `feat` minor,
   `!`/`BREAKING CHANGE` major. Any other change to those inputs still
   takes a patch, so a version always names one image.
3. `check` (fmt, clippy, tests, cargo-deny) and `build`
   (`ghcr.io/hvpaiva/hldr:X.Y.Z`) run in parallel.
4. `deploy` runs `kamal deploy --skip-push`. kamal-proxy switches traffic
   only after `/readyz` answers; otherwise the old container keeps serving.
5. `release` tags `vX.Y.Z` with git-cliff notes and publishes the GitHub
   Release. Tags are the production history.

A daily run retries a release that did not reach production and fails
when `/healthz` disagrees with the last tag.

Rollback, without rebuilding:

```
gh workflow run deploy -f version=X.Y.Z
```

The target is `apollo.hvpaiva.dev`, reached as `deploy` with the host keys
in `config/known_hosts`; a rebuilt host has new keys, so update them there.
App data lives in the `hldr_data` volume. From a
laptop, with a key authorized for `deploy`:

```
bundle install
KAMAL_REGISTRY_PASSWORD=<token with read:packages> bundle exec kamal app details
```

## License

[MIT](LICENSE)
