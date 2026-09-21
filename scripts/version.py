#!/usr/bin/env python3
"""Set or bump the workspace semver in Cargo.toml and Cargo.lock."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
PACKAGES = ("hldr", "hldr-core", "hldr-server")
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")
TOML_VERSION = re.compile(
    r"(?ms)(^\[workspace\.package\]\n(?:^(?!\[).*\n)*?^version = )\"[^\"]+\""
)
LOCK_VERSION = re.compile(
    r'(?m)^(\[\[package\]\]\nname = "{name}"\nversion = )"[^"]+"'
)


def die(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)


def current() -> str:
    text = CARGO_TOML.read_text()
    match = re.search(
        r"(?ms)^\[workspace\.package\]\n(?:^(?!\[).*\n)*?^version = \"([^\"]+)\"",
        text,
    )
    if not match:
        die("workspace.package.version missing in Cargo.toml")
    return match.group(1)


def set_version(version: str) -> None:
    if not SEMVER.match(version):
        die(f"not MAJOR.MINOR.PATCH: {version}")
    toml = CARGO_TOML.read_text()
    toml, n = TOML_VERSION.subn(rf'\1"{version}"', toml, count=1)
    if n != 1:
        die("failed to patch Cargo.toml")
    CARGO_TOML.write_text(toml)

    lock = CARGO_LOCK.read_text()
    for name in PACKAGES:
        lock, n = re.compile(
            LOCK_VERSION.pattern.format(name=re.escape(name))
        ).subn(rf'\1"{version}"', lock, count=1)
        if n != 1:
            die(f"failed to patch Cargo.lock package {name}")
    CARGO_LOCK.write_text(lock)


def bump(kind: str) -> str:
    major, minor, patch = (int(part) for part in current().split("."))
    if kind == "major":
        major, minor, patch = major + 1, 0, 0
    elif kind == "minor":
        minor, patch = minor + 1, 0
    elif kind == "patch":
        patch += 1
    else:
        die(f"unknown bump: {kind}")
    version = f"{major}.{minor}.{patch}"
    set_version(version)
    return version


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("current")
    set_p = sub.add_parser("set")
    set_p.add_argument("version")
    bump_p = sub.add_parser("bump")
    bump_p.add_argument("kind", choices=("patch", "minor", "major"))
    args = parser.parse_args()
    if args.cmd == "current":
        print(current())
    elif args.cmd == "set":
        set_version(args.version)
        print(args.version)
    else:
        print(bump(args.kind))


if __name__ == "__main__":
    main()
