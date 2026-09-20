# hldr

Site and CLI for [hvpaiva.dev](https://hvpaiva.dev). One binary, one SQLite file, one VPS.

```
cargo build --workspace
HLDR_CONTENT_DIR=content cargo run -p hldr-server
hldr version
```

The public listener binds `127.0.0.1:8080` (`HLDR_ADDR` overrides). `/healthz` does not touch the database. SQLite defaults to `./hldr.db`, or `$STATE_DIRECTORY/hldr.db` under systemd. `HLDR_CONTENT_DIR` points at the markdown tree; the server indexes it on boot.

This flake exposes the package and a NixOS module (`nixosModules.hldr`). Machine configuration is not in this repository.
