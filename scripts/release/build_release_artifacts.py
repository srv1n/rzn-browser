#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import platform as platform_module
import shutil
import tarfile
import zipfile
from datetime import datetime, timezone
from pathlib import Path

from release_utils import copy_file, copy_tree, repo_root, rm_rf, run, sha256_file, validate_version


RUNTIME_PLATFORMS = {"linux-x64", "macos-arm64", "macos-x64", "windows-x64"}


def exe_name(name: str, platform_slug: str) -> str:
    return f"{name}.exe" if platform_slug == "windows-x64" else name


def ensure_native_platform(platform_slug: str) -> None:
    system = platform_module.system().lower()
    machine = platform_module.machine().lower()
    expected = {
        "linux-x64": ("linux", ("x86_64", "amd64")),
        "macos-arm64": ("darwin", ("arm64", "aarch64")),
        "macos-x64": ("darwin", ("x86_64", "amd64")),
        "windows-x64": ("windows", ("amd64", "x86_64")),
    }[platform_slug]
    if system != expected[0] or machine not in expected[1]:
        print(
            "[WARN] Building on "
            f"{system}/{machine}, expected {expected[0]}/{','.join(expected[1])} for {platform_slug}."
        )


def write_metadata(stage: Path, *, version: str, kind: str, platform_slug: str | None) -> None:
    manifest = {
        "name": "rzn-browser",
        "version": version,
        "kind": kind,
        "platform": platform_slug,
        "built_at": datetime.now(timezone.utc).isoformat(),
    }
    (stage / "release-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def copy_catalog_payload(root: Path, stage: Path) -> None:
    copy_tree(root / "workflows", stage / "workflows")
    copy_tree(root / "examples" / "browser_automation", stage / "examples" / "browser_automation")
    if (root / "resources").exists():
        copy_tree(root / "resources", stage / "resources")


def build_extension(root: Path, version: str) -> None:
    extension_dir = root / "extension"
    build_env = {"RZN_BUILD_SIGNATURE": f"v{version}"}

    run(["bun", "install", "--frozen-lockfile"], cwd=root / "extension")
    run(
        ["bun", "run", "scripts/generate-types.ts"],
        cwd=extension_dir,
        env=build_env,
    )
    print(f"[INFO] Using RZN_BUILD_SIGNATURE=v{version}")

    rm_rf(extension_dir / "dist")
    (extension_dir / "dist").mkdir(parents=True, exist_ok=True)

    vite_configs = [
        "vite.config.background.ts",
        "vite.config.content.ts",
        "vite.config.shadow-dom.ts",
        "vite.config.pagebridge.ts",
        "vite.config.popup.ts",
    ]
    for config in vite_configs:
        run(["bun", "x", "vite", "build", "--config", config], cwd=extension_dir, env=build_env)

    for source, dest in {
        "background.iife.js": "background.js",
        "contentScript.iife.js": "contentScript.js",
        "shadow-dom-instrumentation.iife.js": "shadow-dom-instrumentation.js",
        "pageBridge.iife.js": "pageBridge.js",
    }.items():
        source_path = extension_dir / "dist" / source
        if source_path.exists():
            source_path.replace(extension_dir / "dist" / dest)

    run(["bun", "scripts/build-ext.ts"], cwd=root, env=build_env)


def build_runtime(root: Path, version: str, platform_slug: str) -> Path:
    ensure_native_platform(platform_slug)
    run(
        [
            "cargo",
            "build",
            "--release",
            "-p",
            "rzn-browser",
            "-p",
            "rzn-browser-worker",
            "-p",
            "rzn-native-host",
        ],
        cwd=root,
    )
    build_extension(root, version)

    stage = root / "dist" / "release" / "stage" / f"rzn-browser-{platform_slug}"
    rm_rf(stage)
    (stage / "bin").mkdir(parents=True, exist_ok=True)

    for binary in ["rzn-browser", "rzn-browser-worker", "rzn-native-host"]:
        src = root / "target" / "release" / exe_name(binary, platform_slug)
        copy_file(src, stage / "bin" / exe_name(binary, platform_slug), executable=True)

    copy_tree(root / "extension" / "dist-chrome", stage / "extension" / "dist-chrome")
    copy_catalog_payload(root, stage)
    if (root / "skills").is_dir():
        copy_tree(root / "skills", stage / "skills")

    copy_file(root / "scripts" / "release" / "install-runtime.sh", stage / "install.sh", executable=True)
    copy_file(root / "scripts" / "release" / "install-runtime.ps1", stage / "install.ps1")
    copy_file(root / "scripts" / "release" / "README.md", stage / "README.md")
    write_metadata(stage, version=version, kind="runtime", platform_slug=platform_slug)
    return stage


def build_workflows(root: Path, version: str) -> Path:
    stage = root / "dist" / "release" / "stage" / "rzn-browser-workflows"
    rm_rf(stage)
    stage.mkdir(parents=True, exist_ok=True)
    copy_catalog_payload(root, stage)
    write_metadata(stage, version=version, kind="workflow-catalog", platform_slug=None)
    return stage


def tar_directory(src: Path, dest: Path) -> None:
    rm_rf(dest)
    with tarfile.open(dest, "w:gz") as archive:
        archive.add(src, arcname=src.name)


def zip_directory(src: Path, dest: Path) -> None:
    rm_rf(dest)
    with zipfile.ZipFile(dest, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(src.rglob("*")):
            archive.write(path, path.relative_to(src.parent))


def write_sidecars(asset: Path, *, version: str, kind: str, platform_slug: str | None) -> None:
    digest = sha256_file(asset)
    asset.with_suffix(asset.suffix + ".sha256").write_text(f"{digest}  {asset.name}\n")
    asset.with_suffix(asset.suffix + ".manifest.json").write_text(
        json.dumps(
            {
                "name": asset.name,
                "version": version,
                "kind": kind,
                "platform": platform_slug,
                "sha256": digest,
                "size": asset.stat().st_size,
            },
            indent=2,
        )
        + "\n"
    )


def package_stage(root: Path, stage: Path, *, version: str, kind: str, platform_slug: str | None) -> Path:
    release_dir = root / "dist" / "release"
    release_dir.mkdir(parents=True, exist_ok=True)
    if kind == "workflow-catalog":
        asset = release_dir / "rzn-browser-workflows.tar.gz"
        tar_directory(stage, asset)
    elif platform_slug == "windows-x64":
        asset = release_dir / f"rzn-browser-{platform_slug}.zip"
        zip_directory(stage, asset)
    else:
        asset = release_dir / f"rzn-browser-{platform_slug}.tar.gz"
        tar_directory(stage, asset)
    write_sidecars(asset, version=version, kind=kind, platform_slug=platform_slug)
    print(f"[OK] Wrote {asset}")
    return asset


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--platform", choices=sorted(RUNTIME_PLATFORMS))
    parser.add_argument("--no-workflows", action="store_true")
    parser.add_argument("--workflows-only", action="store_true")
    args = parser.parse_args()

    validate_version(args.version)
    root = repo_root()
    (root / "dist" / "release").mkdir(parents=True, exist_ok=True)

    if args.workflows_only:
        stage = build_workflows(root, args.version)
        package_stage(root, stage, version=args.version, kind="workflow-catalog", platform_slug=None)
        return

    if not args.platform:
        raise SystemExit("[ERROR] --platform is required unless --workflows-only is set.")

    stage = build_runtime(root, args.version, args.platform)
    package_stage(root, stage, version=args.version, kind="runtime", platform_slug=args.platform)

    if not args.no_workflows:
        workflow_stage = build_workflows(root, args.version)
        package_stage(
            root,
            workflow_stage,
            version=args.version,
            kind="workflow-catalog",
            platform_slug=None,
        )


if __name__ == "__main__":
    main()
