# hldr

Site and CLI for [hvpaiva.dev](https://hvpaiva.dev). One binary, one SQLite file, one VPS.

```
cargo build --workspace
HLDR_CONTENT_DIR=content cargo run -p hldr-server
hldr version
```

The public listener binds `127.0.0.1:8080` (`HLDR_ADDR` overrides). `/healthz` does not touch the database; `/readyz` does. SQLite defaults to `./hldr.db`, or `$STATE_DIRECTORY/hldr.db` under systemd. `HLDR_CONTENT_DIR` points at the markdown tree; the server indexes it on boot. `HLDR_ORIGIN` (default `https://hvpaiva.dev`) is the canonical URL for sitemap, robots, and Open Graph. `curl` gets the text/ANSI version; browsers get HTML.

Production is Kamal on `main` (`config/deploy.yml`). Staging is Kamal on
`staging` (`config/deploy.staging.yml`), at `hldr.apollo.hvpaiva.dev` on
the tailnet. This flake still exposes the package and a NixOS module for
local use; the host does not import them.
