# hldr

Site and CLI for [hvpaiva.dev](https://hvpaiva.dev).

HTML in a browser. JavaScript is not required to read the pages.
Administration is a CLI in the kubectl shape; there is no web panel.

Git is the only durable store. Content and site configuration live in
[hldr-content](https://github.com/hvpaiva/hldr-content): prose as markdown
under a frontmatter, configuration as YAML, every file declaring its
`kind`. The server pulls that repository and materializes one revision of
it into SQLite, which the site reads. Deleting the database loses nothing:
the next sync rebuilds it.

The JSON API is private. It has no authentication of its own: it listens
apart from the site and is reachable only through the tailnet, so the CLI
works from tailnet machines only.

The site is a sample of the craft. A push to `main` that changes the site
is a release: checked, built, deployed, and tagged `vX.Y.Z` once
production is healthy. The tag is the version; nothing is committed back.

## Workspace

```
crates/core       domain, SQLite, markdown
crates/server     hldr-server
crates/cli        hldr
migrations/       sqlx
config/           Kamal: deploy.yml, the target's pinned host keys
cliff.toml        version bumps and release notes (git-cliff)
```

## Run

```
cargo build --workspace
HLDR_CONTENT_DIR=../hldr-content cargo run -p hldr-server
```

Content comes from exactly one source:

- `HLDR_CONTENT_DIR`: a checkout, read as it is on disk.
- `HLDR_CONTENT_REPO` (`owner/name`) with `HLDR_CONTENT_REF` (default
  `main`): the GitHub repository. Each sync resolves the ref to a commit
  and downloads that commit's tarball, so one sync reads one revision.

The server syncs on boot, then every `HLDR_CONTENT_POLL` (default `5m`;
`0` turns polling off), and on `POST /api/v1/sync`. A revision that fails
to index changes nothing: the previous one stays served and the error is
recorded. A failed first sync still serves what the database holds; with
nothing synced yet, `/readyz` answers 503.

Listens on `127.0.0.1:8080`. `HLDR_ADDR` overrides. `/healthz` does not
touch the database; `/readyz` does, and adds the served content revision.
Both report `version` and `revision`. SQLite defaults to `./hldr.db`.
`HLDR_ORIGIN` sets the canonical URL and defaults to the local listener.

The private API (`/api/v1`, and `/healthz`) listens on `127.0.0.1:8081`.
`HLDR_API_ADDR` overrides, as `host:port` or `unix:/absolute/path`. A unix
socket path is taken over from whatever process held it, so a new container
can boot while the old one still serves; a non-socket file at the path is
an error. `GET /api/v1/sync` shows the sync state; `POST` runs a sync and
answers once it is done: 422 when the content is invalid, 502 when it
could not be fetched.


```
docker build -t hldr .
docker run --rm -p 8080:8080 -e HLDR_CONTENT_REPO=hvpaiva/hldr-content hldr
```

Outside the release pipeline the version is `dev`. Cargo manifests carry
no version; `HLDR_VERSION` and `HLDR_REVISION` stamp it at build time.

## CLI

`hldr` reads the private API, so it works from the tailnet. It finds the
server in `--server`, then `HLDR_SERVER`, then `$XDG_CONFIG_HOME/hldr/config.yaml`
(`HLDR_CONFIG` names another file):

```yaml
server: https://apollo.<tailnet>.ts.net:8443
```

```
cargo install --path crates/cli
hldr api-resources
hldr get projects -o wide
hldr get theme nord retro-82 -o yaml
hldr get p -o jsonpath='{.items[*].metadata.name}'
hldr describe project hldr
hldr explain project.spec.status
hldr version
```

Resource types, their short names and their table columns come from the
server (`/api/v1/api-resources`), cached per server under
`$XDG_CACHE_HOME/hldr/` for six hours and refreshed at once for a type the
cache does not know. `-o` takes `table`, `wide`, `json`, `yaml`, `name`,
`jsonpath=TEMPLATE` (kubectl templates without `range`) and
`custom-columns=HEADER:PATH,...`.

## Test

```
cargo fmt --all --check
cargo clippy --all-targets --workspace -- -D warnings
cargo test --workspace
```

## Deploy

`.github/workflows/deploy.yml`, on every push to `main`:

1. `plan` compares the deployable inputs (`crates/`, `migrations/`, Cargo
   files, `Dockerfile`, `config/`, Kamal gems) with the last `vX.Y.Z` tag.
   Nothing changed: nothing runs. Content is not an input: it ships from
   hldr-content through the sync, not through a release.
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
App data lives in the `hldr_data` volume. The private API is the socket
`/run/tailnet/8443.sock`, which apollo serves at
`https://apollo.<tailnet>.ts.net:8443` to the tailnet only. From a
laptop, with a key authorized for `deploy`:

```
bundle install
KAMAL_REGISTRY_PASSWORD=<token with read:packages> bundle exec kamal app details
```

## License

[MIT](LICENSE)
