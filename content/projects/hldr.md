---
title: hldr
tagline: Site and CLI for hvpaiva.dev. HTML for browsers, text for curl.
status: active
highlight: 1
tags: [rust, sqlite]
links:
  repo: https://github.com/hvpaiva/hldr
github: hvpaiva/hldr
---

## What it is

Personal site. Content lives as markdown in git; runtime state lives in
SQLite. The image lives in the registry. Administration is a CLI in the
kubectl shape, not a web panel.

The public surface is HTML rendered on the server. No JavaScript is
required to read it. `curl hvpaiva.dev` is a first-class interface.

## Why it exists

A site that is also a sample of the craft. If the headers are sloppy or
the deploy is a ritual, it contradicts the résumé. `main` is production.

## Decisions

SQLite opened in WAL, indexed on boot. Desired state is files; observed
state is the database. YAML is `serde-saphyr` because the old serde_yaml
world stopped.

## What went wrong

The first healthcheck returned 503.
