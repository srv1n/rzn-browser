#!/usr/bin/env python3
"""Fail fast on a workflow catalog that would not load.

The catalog ships without a Rust build, so this is the only gate between a bad
JSON edit and a published bundle. It checks what the runtime loader needs, not
the whole manifest schema.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

from release_utils import repo_root

CONTRACT = "rzn.workflow_manifest"
REQUIRED_FIELDS = ("id", "name", "version", "system", "capability")


def side_effect_classes(value: object) -> set[str]:
    """Manifests write side effects either as class strings or as objects with a
    `class` field. Both forms mean the same thing to the loader."""
    classes: set[str] = set()
    for item in value or []:
        if isinstance(item, str):
            classes.add(item)
        elif isinstance(item, dict) and isinstance(item.get("class"), str):
            classes.add(item["class"])
    return classes


def main() -> None:
    workflows = repo_root() / "workflows"
    errors: list[str] = []
    manifests = 0

    for path in sorted(workflows.rglob("*.json")):
        rel = path.relative_to(workflows.parent)
        try:
            data = json.loads(path.read_text())
        except json.JSONDecodeError as exc:
            errors.append(f"{rel}: invalid JSON ({exc})")
            continue

        if not isinstance(data, dict) or data.get("schema_version") != CONTRACT:
            continue

        manifests += 1
        missing = [field for field in REQUIRED_FIELDS if not data.get(field)]
        if missing:
            errors.append(f"{rel}: missing required field(s): {', '.join(missing)}")

        steps = data.get("steps")
        if steps is not None and (not isinstance(steps, list) or not steps):
            errors.append(f"{rel}: 'steps' must be a non-empty list when present")
            continue

        declared = side_effect_classes(data.get("side_effects"))
        for index, step in enumerate(steps or []):
            if not isinstance(step, dict) or not step.get("id"):
                errors.append(f"{rel}: steps[{index}] has no id")
                continue
            action = step.get("action")
            if not isinstance(action, dict) or not action.get("kind"):
                errors.append(f"{rel}: steps[{index}] has no action.kind")
                continue
            undeclared = sorted(side_effect_classes(action.get("side_effects")) - declared)
            if undeclared:
                errors.append(
                    f"{rel}: step '{step['id']}' declares undeclared side effect(s): "
                    + ", ".join(undeclared)
                )

    if errors:
        for error in errors:
            print(f"[ERROR] {error}", file=sys.stderr)
        raise SystemExit(f"[ERROR] {len(errors)} catalog problem(s) found.")

    print(f"[OK] Checked {manifests} workflow manifest(s).")


if __name__ == "__main__":
    main()
