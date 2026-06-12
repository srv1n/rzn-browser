#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import os
import re
import shutil
import subprocess
from pathlib import Path
from typing import Iterable


VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def validate_version(version: str) -> None:
    if not VERSION_RE.match(version):
        raise SystemExit(f"[ERROR] Invalid version '{version}'. Expected major.minor.patch.")


def run(cmd: Iterable[str], *, cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    display = " ".join(str(part) for part in cmd)
    print(f"[INFO] {display}")
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    subprocess.run(list(cmd), cwd=cwd, env=merged_env, check=True)


def output(cmd: Iterable[str], *, cwd: Path | None = None) -> str:
    return subprocess.check_output(list(cmd), cwd=cwd, text=True).strip()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def rm_rf(path: Path) -> None:
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    elif path.exists() or path.is_symlink():
        path.unlink()


def copy_tree(src: Path, dest: Path, *, ignore: shutil.IgnorePattern | None = None) -> None:
    rm_rf(dest)
    shutil.copytree(src, dest, ignore=ignore)


def copy_file(src: Path, dest: Path, *, executable: bool = False) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dest)
    if executable:
        dest.chmod(dest.stat().st_mode | 0o755)
