# hldr

Site and CLI for [hvpaiva.dev](https://hvpaiva.dev). One binary, one SQLite file, one VPS.

```
cargo build --workspace
cargo run -p hldr-server
hldr version
```

The public listener binds `127.0.0.1:8080` (`HLDR_ADDR` overrides). `/healthz` is the readiness probe.

This flake exposes the package and a NixOS module (`nixosModules.hldr`). Machine configuration is not in this repository.
