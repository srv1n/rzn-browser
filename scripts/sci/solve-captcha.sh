#!/usr/bin/env bash
# Read the sci.gov.in CAPTCHA image and print the answer on stdout.
#   scripts/sci/solve-captcha.sh /path/to/captcha.png
# SCI_SOLVER=claude (default) uses `claude -p` on Haiku; SCI_SOLVER=codex uses `codex exec` on gpt-6-luna.
set -euo pipefail

img="$1"
prompt="The image $img is a CAPTCHA that shows a short arithmetic question, such as '10 - 3'. Compute the answer. Reply with only the answer, no other text."

case "${SCI_SOLVER:-claude}" in
  claude)
    out="$(cd "$(dirname "$img")" && printf "%s" "$prompt" | claude -p --model haiku --allowedTools Read)" ;;
  codex)
    out="$(codex exec --ephemeral --skip-git-repo-check -s read-only -m gpt-6-luna -c model_reasoning_effort=xhigh \
      -i "$img" "$prompt" 2>/dev/null)" ;;
  *) echo "unknown SCI_SOLVER: $SCI_SOLVER" >&2; exit 64 ;;
esac

# The answer is the last integer the model printed.
ans="$(printf '%s\n' "$out" | grep -oE -- '-?[0-9]+' | tail -n 1 || true)"
[[ -n "$ans" ]] || { echo "solver gave no number: $out" >&2; exit 1; }
echo "$ans"
