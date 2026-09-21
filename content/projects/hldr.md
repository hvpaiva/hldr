---
title: hldr
tagline: Site and CLI for hvpaiva.dev. One binary, one SQLite file, one VPS.
status: active
highlight: 1
tags: [rust, nixos, sqlite]
links:
  repo: https://github.com/hvpaiva/hldr
github: hvpaiva/hldr
---

## What it is

Personal site served with Kamal on a NixOS host. Content lives as markdown
in git and ships in the image; runtime state lives in SQLite.
Administration is a CLI in the kubectl shape, not a web panel.

The public surface is HTML rendered on the server. No JavaScript is
required to read it. `curl hvpaiva.dev` is a first-class interface.

## Why it exists

A site that is also a sample of the craft. If the headers are sloppy or
the deploy is a ritual, it contradicts the résumé. A merge to `main` is
what turns production.

## Decisions

SQLite opened in WAL, indexed on boot. Desired state is files; observed
state is the database. YAML is `serde-saphyr` because the old serde_yaml
world stopped.

## What went wrong

The first healthcheck returned 503 and the rollback path fought
deploy-rs when the waiter ran on the host itself. Cursor injected
`Co-authored-by` into commits until a machine-wide hook stripped it.
The host config lived in the public repo until that was cut out of
history.
