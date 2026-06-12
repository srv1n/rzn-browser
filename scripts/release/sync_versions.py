#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

from release_utils import repo_root, validate_version


CRATE_MANIFESTS = [
    "crates/rzn_browser/Cargo.toml",
    "crates/rzn_native_host/Cargo.toml",
    "crates/rzn_core/Cargo.toml",
    "crates/rzn_contracts/Cargo.toml",
    "crates/rzn_plan/Cargo.toml",
    "crates/rzn_sdk/Cargo.toml",
    "crates/rzn_plugin_devkit/Cargo.toml",
]

JSON_VERSION_FILES = [
    ("extension/package.json", ("version",)),
    ("extension/src/manifest.base.json", ("version", "version_name")),
    ("scripts/plugins/config/rzn-browser.json", ("version",)),
]


def replace_cargo_version(path: Path, version: str) -> bool:
    original = path.read_text()
    updated, count = re.subn(
        r'(?m)^version\s*=\s*"[^"]+"',
        f'version = "{version}"',
        original,
        count=1,
    )
    if count != 1:
        raise SystemExit(f"[ERROR] Could not update package version in {path}")
    if updated == original:
        return False
    path.write_text(updated)
    return True


def replace_json_versions(path: Path, keys: tuple[str, ...], version: str) -> bool:
    payload = json.loads(path.read_text())
    changed = False
    for key in keys:
        if payload.get(key) != version:
            payload[key] = version
            changed = True
    if changed:
        path.write_text(json.dumps(payload, indent=2) + "\n")
    return changed


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--print-files", action="store_true")
    args = parser.parse_args()

    validate_version(args.version)
    root = repo_root()
    changed: list[str] = []

    for rel in CRATE_MANIFESTS:
        path = root / rel
        if path.exists() and replace_cargo_version(path, args.version):
            changed.append(rel)

    for rel, keys in JSON_VERSION_FILES:
        path = root / rel
        if path.exists() and replace_json_versions(path, keys, args.version):
            changed.append(rel)

    if args.print_files:
        for rel in changed:
            print(rel)


if __name__ == "__main__":
    main()
