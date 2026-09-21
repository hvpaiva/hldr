# hldr

[hvpaiva.dev](https://hvpaiva.dev) — HTML in a browser, text if you `curl`.

Content is markdown in git. What the process observes lives in SQLite.
Administration is a CLI in the kubectl shape, not a web panel.

```
cargo build --workspace
HLDR_CONTENT_DIR=content cargo run -p hldr-server
```
