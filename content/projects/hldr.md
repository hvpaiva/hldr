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

Personal site served as a systemd unit on a NixOS host. Content lives as
markdown in git; runtime state lives in SQLite. Administration is a CLI
in the kubectl shape, not a web panel.

The public surface is HTML rendered on the server. No JavaScript is
required to read it.
