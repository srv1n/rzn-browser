#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path

from release_utils import repo_root


def git(args: list[str], root: Path) -> str:
    return subprocess.check_output(["git", *args], cwd=root, text=True).strip()


def try_git(args: list[str], root: Path) -> str | None:
    try:
        value = subprocess.check_output(
            ["git", *args],
            cwd=root,
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except subprocess.CalledProcessError:
        return None
    return value or None


def commit_range(tag: str, root: Path) -> tuple[str | None, str]:
    previous = try_git(["describe", "--tags", "--abbrev=0", f"{tag}^"], root)
    if previous:
        return previous, f"{previous}..{tag}"
    return None, tag


def changed_files(range_spec: str, root: Path) -> list[str]:
    raw = try_git(["diff-tree", "--no-commit-id", "--name-only", "-r", range_spec], root)
    if not raw:
        raw = try_git(["show", "--pretty=", "--name-only", "--format=", range_spec], root)
    if not raw:
        return []
    return sorted({line for line in raw.splitlines() if line.strip()})


def commits(range_spec: str, root: Path) -> list[str]:
    raw = try_git(["log", "--format=%s", range_spec], root)
    if not raw:
        return []
    return [line.strip() for line in raw.splitlines() if line.strip()]


def section_for_file(path: str) -> str:
    if path.startswith(".github/") or path.startswith("scripts/release/"):
        return "Release and install"
    if path.startswith("install.") or path.startswith("setup.sh"):
        return "Release and install"
    if path.startswith("extension/"):
        return "Extension"
    if path.startswith("crates/"):
        return "Runtime"
    if path.startswith("workflows/") or path.startswith("examples/browser_automation/"):
        return "Workflow catalog"
    if path.startswith("docs/"):
        return "Docs"
    return "Project"


def summarize_files(files: list[str]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for path in files:
        section = section_for_file(path)
        counts[section] = counts.get(section, 0) + 1
    return dict(sorted(counts.items()))


def render_notes(tag: str, version: str, previous_tag: str | None, commit_subjects: list[str], files: list[str]) -> str:
    counts = summarize_files(files)
    lines = [
        f"# RZN Browser {tag}",
        "",
        "This release publishes installable RZN Browser runtime bundles and the workflow catalog.",
        "",
        "## Highlights",
    ]

    if counts:
        for section, count in counts.items():
            lines.append(f"- {section}: {count} changed file{'s' if count != 1 else ''}")
    else:
        lines.append("- Release artifacts rebuilt from the tagged source tree")

    lines.extend(["", "## Install"])
    lines.extend(
        [
            "- macOS/Linux: `curl -fsSL https://raw.githubusercontent.com/srv1n/rzn-browser/main/install.sh | sh`",
            "- Windows: `irm https://raw.githubusercontent.com/srv1n/rzn-browser/main/install.ps1 | iex`",
        ]
    )

    if commit_subjects:
        lines.extend(["", "## Commit Summary"])
        for subject in commit_subjects[:40]:
            lines.append(f"- {subject}")
        if len(commit_subjects) > 40:
            lines.append(f"- Plus {len(commit_subjects) - 40} additional commits")

    lines.extend(
        [
            "",
            "## Release Metadata",
            f"- Version: `{version}`",
            f"- Previous tag: `{previous_tag or 'none'}`",
        ]
    )
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--metadata-output", required=True)
    args = parser.parse_args()

    root = repo_root()
    tag = args.tag
    version = tag.removeprefix("v")
    previous_tag, range_spec = commit_range(tag, root)
    commit_subjects = commits(range_spec, root)
    files = changed_files(range_spec, root)

    output = root / args.output
    metadata_output = root / args.metadata_output
    output.parent.mkdir(parents=True, exist_ok=True)
    metadata_output.parent.mkdir(parents=True, exist_ok=True)

    output.write_text(render_notes(tag, version, previous_tag, commit_subjects, files))
    metadata_output.write_text(
        json.dumps(
            {
                "tag": tag,
                "version": version,
                "previous_tag": previous_tag,
                "commit_count": len(commit_subjects),
                "changed_file_count": len(files),
                "generated_at": datetime.now(timezone.utc).isoformat(),
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
