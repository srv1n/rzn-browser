#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SKILL_NAME="rzn-workflow-builder"
SOURCE_DIR="${REPO_ROOT}/skills/${SKILL_NAME}"

usage() {
  cat <<EOF
Usage: $(basename "$0") [--global | --project | --path PATH] [--force]

Install the bundled ${SKILL_NAME} skill.

Targets:
  --global         Install into \$CODEX_HOME/skills or ~/.codex/skills
  --project        Install into <repo>/.codex/skills
  --path PATH      Install into a custom skills directory

Flags:
  --force          Replace an existing installed copy without prompting
  -h, --help       Show this help

If no target is passed, the script prompts for:
  1) global install
  2) project-level install
EOF
}

prompt_target() {
  echo "Install ${SKILL_NAME} where?"
  echo "1) global  (\$CODEX_HOME/skills or ~/.codex/skills)"
  echo "2) project (${REPO_ROOT}/.codex/skills)"
  printf "Choose 1 or 2 [1]: "
  read -r choice
  case "${choice:-1}" in
    1) TARGET_MODE="global" ;;
    2) TARGET_MODE="project" ;;
    *) echo "[ERROR] Invalid choice: ${choice}" >&2; exit 1 ;;
  esac
}

copy_skill() {
  local src="$1"
  local dest="$2"
  rm -rf "$dest"
  mkdir -p "$(dirname "$dest")"
  cp -R "$src" "$dest"
  find "$dest/scripts" -type f -name "*.sh" -exec chmod +x {} + 2>/dev/null || true
}

TARGET_MODE=""
TARGET_ROOT=""
FORCE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --global)
      TARGET_MODE="global"
      shift
      ;;
    --project)
      TARGET_MODE="project"
      shift
      ;;
    --path)
      TARGET_MODE="custom"
      TARGET_ROOT="${2:?missing path after --path}"
      shift 2
      ;;
    --force)
      FORCE=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "[ERROR] Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ ! -d "$SOURCE_DIR" ]]; then
  echo "[ERROR] Missing source skill directory: $SOURCE_DIR" >&2
  exit 1
fi

if [[ -z "$TARGET_MODE" ]]; then
  prompt_target
fi

case "$TARGET_MODE" in
  global)
    TARGET_ROOT="${CODEX_HOME:-$HOME/.codex}/skills"
    ;;
  project)
    TARGET_ROOT="${REPO_ROOT}/.codex/skills"
    ;;
  custom)
    ;;
  *)
    echo "[ERROR] Unsupported target mode: ${TARGET_MODE}" >&2
    exit 1
    ;;
esac

DEST_DIR="${TARGET_ROOT}/${SKILL_NAME}"
if [[ -e "$DEST_DIR" && "$FORCE" != "1" ]]; then
  printf "Replace existing %s at %s? [y/N]: " "$SKILL_NAME" "$DEST_DIR"
  read -r confirm
  case "${confirm:-n}" in
    y|Y|yes|YES) ;;
    *) echo "[INFO] Aborted."; exit 0 ;;
  esac
fi

copy_skill "$SOURCE_DIR" "$DEST_DIR"

echo "[OK] Installed ${SKILL_NAME} to ${DEST_DIR}"

case "$TARGET_MODE" in
  global)
    cat <<EOF

Success.

- Scope: global
- Skill path: ${DEST_DIR}
- Codex should auto-discover this skill from your global skills directory.
- Use it by asking for \$${SKILL_NAME} or by pointing another agent at this path.
EOF
    ;;
  project)
    cat <<EOF

Success.

- Scope: project
- Skill path: ${DEST_DIR}
- This keeps the skill with the repo.
- If your agent does not auto-discover project-local skills, reference the path directly:
  ${DEST_DIR}
EOF
    ;;
  custom)
    cat <<EOF

Success.

- Scope: custom
- Skill path: ${DEST_DIR}
- Reference this path directly if your agent does not auto-discover custom skill directories.
EOF
    ;;
esac
