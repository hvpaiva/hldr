# hldr

Site and CLI for [hvpaiva.dev](https://hvpaiva.dev).

HTML in a browser. JavaScript is not required to read the pages.
Administration is a CLI in the kubectl shape; there is no web panel.

Git is the only durable store. Content and site configuration live in
[hldr-content](https://github.com/hvpaiva/hldr-content) as YAML manifests
in the kubectl shape, `kind`, `metadata` and `spec`, and the markdown each
page names beside its manifest. Pages, the file tree and page types are
content too: a new type, such as a blog's posts, is a manifest, not a
release. The server pulls that repository and materializes one revision of
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
crates/core       manifests, templates, SQLite
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

That is also the preview of a content change: with a checkout of
hldr-content next to this one, a short poll picks up every save, and an
invalid file leaves the last good state served with the error on `/health`:

```
HLDR_CONTENT_DIR=../hldr-content HLDR_CONTENT_POLL=2s cargo run -p hldr-server
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
Both report `version` and `revision`. `/health` is the page behind
`:checkhealth` and the version in the statusline: server version and
revision, the served content revision, and how the last sync went,
rendered without JavaScript. SQLite defaults to `./hldr.db`.
`HLDR_ORIGIN` sets the canonical URL and defaults to the local listener.

The private API (`/api/v1`, and `/healthz`) listens on `127.0.0.1:8081`.
`HLDR_API_ADDR` overrides, as `host:port` or `unix:/absolute/path`. A unix
socket path is taken over from whatever process held it, so a new container
can boot while the old one still serves; a non-socket file at the path is
an error. `GET /api/v1/sync` shows the sync state; `POST` runs a sync and
answers once it is done: 422 when the content is invalid, 502 when it
could not be fetched. The server reads GitHub anonymously, 60 requests an
hour per IP; a spent limit is recorded with the time it resets, and
`HLDR_GITHUB_TOKEN` (read-only) raises it.

The server records what happens to it as events: `Started` and `Stopped`,
`Synced` when what it serves changes, `SyncFailed` and `RateLimited`. The
same event again right after itself is counted rather than added, and
events are kept 30 days in the database; unlike content, they are not
rebuilt from git. `GET /api/v1/events` lists them oldest first with the
stream position it read them at, and `?after=SEQ` answers only what was
written after it, updates included. `GET /api/v1/server` reports the
version, uptime, content state, database size and the GitHub rate limit
as its last answer reported it. Both are private: the site serves neither.

The site counts what it serves without tracking anyone: no cookie, no
script, and no address or user agent stored. Each page answered and each
404 (all in one `404` route) is counted per UTC day, with the referring
host; a user agent that names itself a tool or crawler counts only as a
bot. A visitor is a hash of the day's random salt, the client's address
(the last `X-Forwarded-For` entry, which kamal-proxy writes) and its user
agent. The first flush after a day ends counts that day's visitors and
deletes its hashes and salt, so nobody can be followed across days, and
visitors over a period are the sum of each day's. Counts are flushed every
minute and on shutdown; both servers of a deploy can flush into the same
database. `GET /api/v1/metrics/daily`, `/pages` and `/referrers`, with
`?since=7d`, `30d` or `all`, are private too. Unlike content, metrics are
not rebuilt from git: losing the database loses them.


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

Each release attaches a static Linux binary, `hldr-x86_64-unknown-linux-musl`,
with its checksum:

```
gh release download --repo hvpaiva/hldr --pattern 'hldr-x86_64-unknown-linux-musl*'
sha256sum -c hldr-x86_64-unknown-linux-musl.sha256
install -m 755 hldr-x86_64-unknown-linux-musl ~/.local/bin/hldr
```

`hldr version` warns when it and the server come from different releases.
From source: `cargo install --locked --path crates/cli`.

```
hldr api-resources
hldr get projects -o wide
hldr get theme nord retro-82 -o yaml
hldr get p -o jsonpath='{.items[*].metadata.name}'
hldr describe project hldr
hldr explain project.metadata.status
hldr version
hldr get events --watch             # list, then print what happens
hldr describe server                # uptime, content, database, GitHub quota, events
hldr top                            # views, visitors and bots: the period, then per day
hldr top pages --since 30d          # most viewed routes
hldr top referrers --since all      # hosts that sent visitors
```

`top` takes `--since` (7d by default; any number of days, or `all`),
`--limit` (the most viewed pages or hosts, 20 by default, or the latest
days) and `-o json|yaml`. The last minute of counts may not be flushed yet.

`--watch` asks for what changed every two seconds and never pages; a
server that stops answering, as during a deploy, is reported once and
waited for.

Writes go to the content repository the server syncs from, which the
server reports, so the CLI can never write where the site does not read.
A page's markdown travels with its manifest: `apply` commits the one
beside it, `delete` removes both, and `edit --content` opens the markdown
instead of the manifest:

```
hldr edit project hldr              # $HLDR_EDITOR, $VISUAL or $EDITOR
hldr edit page about --content
hldr apply -f projects/atlas.yaml   # kind from the file, name from its file name
hldr apply -f ~/dev/hldr-content    # a checkout, file by file
hldr diff -f projects/atlas.yaml    # exit 1 when applying would change something
hldr patch collection blog -p '{"spec":{"enabled":true}}'
hldr patch project hldr -p '{"metadata":{"tagline":"New line"}}'
hldr delete project old-thing
hldr sync                           # fetch now instead of on the next poll
hldr sync status                    # served revision against the branch head
hldr history                        # the branch's commits, the served one marked
hldr history projects/atlas.yaml    # only those that changed a file
hldr history -r f3d3461             # one commit: message and files changed
```

`history` reads the content repository's commits from GitHub, as
`kubectl rollout history` lists revisions, with the token when there is
one (`--limit`, at most 100; `-o json|yaml`). It never writes.

`hldr validate -f PATH` needs no server or network, and reads the config
file only for colors: it checks
files with the parser the server indexes with and, for a directory, the
checks across files too: the singletons and the pages the site needs
exist, page types and collections pair up, names do not clash, and every
reference in a template resolves. A manifest inside a checkout is checked
against the page types it declares. hldr-content runs it on every push.

```
hldr validate -f ~/dev/hldr-content
hldr validate -f ~/dev/hldr-content/projects/atlas.yaml
```

Each writing command validates with the parser the server indexes with, then
commits every change at once on top of the commit it read; if the branch
moved meanwhile, nothing is written. After committing, it asks the server
to sync that exact commit and returns once the site shows it (`--no-sync`
leaves it to the next poll). A failed edit reopens with the error written
into the file, as `kubectl edit` does.

Writing needs a GitHub token with Contents write access to the content
repository only. `HLDR_GITHUB_TOKEN` holds one, or a command prints it,
run only when something is about to be written:

```yaml
server: https://apollo.<tailnet>.ts.net:8443
content:
  token_command: [op, read, "op://Private/hldr-content/credential"]
```

Resource types, their short names and their table columns come from the
server (`/api/v1/api-resources`): the built-in kinds, and one per page type
content declares, so a type added in hldr-content works here without a new
`hldr`. Discovery is cached per server under `$XDG_CACHE_HOME/hldr/` for six
hours, and refreshed when the server runs another version or serves another
content revision, and at once for a type the cache does not know. `-o`
takes `table`, `wide`, `json`, `yaml`, `name`, `jsonpath=TEMPLATE` (kubectl
templates without `range`) and `custom-columns=HEADER:PATH,...`.

### Colors

Output is colored as [kubecolor](https://github.com/kubecolor/kubecolor)
colors kubectl's, with its presets, theme keys and color syntax. Tables
color the header and cycle a color per column, and color an enum field's
column as a status: its `ok` values as `status.success`, its other values
as `status.warning`, as the site draws them; `describe`, `explain` and
`version` color keys by depth; `-o json` and `-o yaml` color keys by depth
and values by type; `diff` colors added and removed lines; `apply`,
`patch` and `delete` color what happened, and every write its commit and
sync; errors and warnings go red and yellow; `--help` and usage errors
follow the theme too. Uncolored output is byte for byte what it was before.

Each stream is colored only when it is a terminal, so pipes and
redirected files stay plain. `--plain` or `NO_COLOR` turns colors off;
`--force-colors[=LEVEL]` (or `HLDR_FORCE_COLORS`, or `FORCE_COLOR`)
colors anyway, with `auto`, `basic`, `256` or `truecolor` colors, and
`none` never colors. A color the terminal cannot show is drawn as the
nearest one it can: `COLORTERM=truecolor` gives 24-bit, a `TERM` with
`256color` gives 256, anything else the 16 basic ones.

The preset comes from `--color-preset`, `HLDR_COLOR_PRESET` or
`color.preset`: `auto` (the default), `dark`, `light`, `none`, or
`protanopia`, `deuteranopia` or `tritanopia`, alone or with `-dark` or
`-light`. `auto` picks light when `COLORFGBG` says the background is
light, dark otherwise; `--light-background` (or `HLDR_LIGHT_BACKGROUND`)
takes the light variant of any preset. `color.theme` sets any key over
the preset:

```yaml
color:
  preset: dark
  theme:
    base:
      danger: fg=white:bg=red:bold
    table:
      columns: [cyan, "#6afd6a", 208]
```

`HLDR_COLOR_THEME_BASE_DANGER=red` sets a key from the environment. A
color is `:`-separated fields: a name (`red`, `hicyan`, `gray`), an
attribute (`bold`, `italic`, `underline`...), `fg=` or `bg=`, a 256-color
code, `#rrggbb` or `rgb(r, g, b)`; `none` is no styling. List keys take
several, as a YAML list or split by `/`.

An unset key takes the value of the key it falls back to, so setting a
`base` key recolors everything built on it:

| Key | Falls back to |
| --- | --- |
| `base.info`, `base.primary`, `base.secondary`, `base.success`, `base.warning`, `base.danger`, `base.muted` | the preset |
| `base.key` (list) | `base.secondary` |
| `data.key` (list), `describe.key`, `explain.key`, `version.key` | `base.key` |
| `data.string` | `base.info` |
| `data.true`, `status.success`, `diff.added`, `apply.created`, `sync.committed`, `sync.synced` | `base.success` |
| `data.false`, `status.error`, `stderr.error`, `explain.required`, `diff.removed`, `delete.deleted` | `base.danger` |
| `data.number`, `apply.unchanged` | `base.primary` |
| `data.null`, `diff.unchanged`, `help.placeholder` | `base.muted` |
| `status.warning`, `stderr.warning`, `apply.configured`, `patch.patched`, `sync.skipped` | `base.warning` |
| `table.header` | `base.info` |
| `help.header` | `table.header` |
| `table.columns` (list) | `base.info`, `base.secondary` |
| `apply.dryrun`, `help.flag` | `base.secondary` |

`stderr.warning`, `help.placeholder` and the `sync` keys are hldr's own:
the `sync` keys color `committed`, `synced` and `not synced`, and
`help.placeholder` a flag's `<VALUE>` in `--help`. Every other key means
what it means in kubecolor.

### Paging

As in kubecolor, paging is opt-in. `--paging` (or `HLDR_PAGING=auto`, or
`paging: auto` in the config file) sends output through a pager when
stdout is a terminal; `--no-paging` turns it off for one run. The pager
is `--pager` (or `HLDR_PAGER`), then `pager:` in the config file, then
`PAGER`, then `less -RF` or `more` when they are on the `PATH`. Its
command line is split on whitespace and run without a shell, and it
must pass escapes through, as `less -R` does, to keep the colors:

```yaml
paging: auto
pager: less -RF
```

The pager starts with the first line of output, so a command that prints
nothing starts none, and a token command asking to unlock runs before
it. Errors and warnings wait for the pager to exit, so none lands inside
it. `edit` is never paged.

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
3. `check` (fmt, clippy, tests, cargo-deny), `build`
   (`ghcr.io/hvpaiva/hldr:X.Y.Z`) and `cli` (the static `hldr`) run in
   parallel.
4. `deploy` runs `kamal deploy --skip-push`. kamal-proxy switches traffic
   only after `/readyz` answers; otherwise the old container keeps serving.
5. `release` tags `vX.Y.Z` with git-cliff notes and publishes the GitHub
   Release with the CLI attached. Tags are the production history.

A daily run retries a release that did not reach production and fails
when `/healthz` disagrees with the last tag.

Rollback, without rebuilding:

```
gh workflow run deploy -f version=X.Y.Z
```

A version older than a migration the database has applied refuses to open
it, and a version before 4.0.0 cannot read today's content layout. Going
back across either means content that version reads and a fresh database,
which its first sync rebuilds.

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
