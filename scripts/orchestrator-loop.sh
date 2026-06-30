#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage:
  scripts/orchestrator-loop.sh PROMPT...

Runs fresh Codex exec sessions until the final agent message does not end with
the continue signal.

This script must run inside the gptpod shell sandbox (`gptpod -s`).

Environment:
  VIP9R_ORCHESTRATOR_SIGNAL    Continue signal. Default: VIP9R_ORCHESTRATOR_CONTINUE
  VIP9R_ORCHESTRATOR_MAX_RUNS  Safety cap. 0 means unlimited. Default: 0
  VIP9R_CODEX_SANDBOX          Codex sandbox mode. Default: danger-full-access
  VIP9R_CODEX_APPROVAL_POLICY  Codex approval_policy override. Default: never

Stop marker:
  temp/orchestrator-loop.stop   If present, exit instead of starting or continuing.
EOF
}

if [[ $# -eq 0 || "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  if [[ $# -eq 0 ]]; then
    exit 2
  fi
  exit 0
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd -- "$script_dir/.." && pwd)"
codex_events="$repo/scripts/codex-events.py"
stop_marker_rel="temp/orchestrator-loop.stop"
stop_marker="$repo/$stop_marker_rel"

require_gptpod_sandbox() {
  local project_marker="/run/claudepod-project"
  local mode_marker="/run/claudepod-mode"
  local project
  local mode

  if [[ ! -r "$project_marker" || ! -r "$mode_marker" ]]; then
    echo "orchestrator-loop: not running inside the gptpod shell sandbox" >&2
    echo "orchestrator-loop: start it with: gptpod -s" >&2
    exit 2
  fi

  project="$(<"$project_marker")"
  mode="$(<"$mode_marker")"

  if [[ "$mode" != "shell" ]]; then
    echo "orchestrator-loop: expected gptpod shell mode, got: $mode" >&2
    echo "orchestrator-loop: start it with: gptpod -s" >&2
    exit 2
  fi

  if [[ "$project" != "$repo" ]]; then
    echo "orchestrator-loop: gptpod project mismatch" >&2
    echo "orchestrator-loop: /run/claudepod-project=$project" >&2
    echo "orchestrator-loop: script repo=$repo" >&2
    exit 2
  fi
}

require_gptpod_sandbox

signal="${VIP9R_ORCHESTRATOR_SIGNAL:-VIP9R_ORCHESTRATOR_CONTINUE}"
max_runs="${VIP9R_ORCHESTRATOR_MAX_RUNS:-0}"
sandbox="${VIP9R_CODEX_SANDBOX:-danger-full-access}"
approval_policy="${VIP9R_CODEX_APPROVAL_POLICY:-never}"

if ! [[ "$max_runs" =~ ^[0-9]+$ ]]; then
  echo "VIP9R_ORCHESTRATOR_MAX_RUNS must be a non-negative integer" >&2
  exit 2
fi

base_prompt="$*"
script_instructions="$(cat <<EOF

## Wrapper note

This run is managed by a loop script. If more autonomous work should continue,
end your normal final report with a line containing only:

$signal

The next run will be a fresh session in this working directory. Leave durable
handoff state in files, VCS, or project docs. Use temp/loop_state.md to
communicate retry state. Do not add the signal when the objective is complete,
user attention is needed, or automatic continuation is unsafe.
EOF
)"
prompt="$base_prompt
$script_instructions"

run=0
while :; do
  if [[ -e "$stop_marker" ]]; then
    echo "orchestrator-loop: stop marker present: $stop_marker_rel" >&2
    exit 0
  fi

  run=$((run + 1))
  if (( max_runs > 0 && run > max_runs )); then
    echo "orchestrator-loop: reached VIP9R_ORCHESTRATOR_MAX_RUNS=$max_runs" >&2
    exit 124
  fi

  echo "── orchestrator-loop: codex exec run $run ──" >&2

  final_message="$(mktemp -t vip9r-orchestrator-final.XXXXXX)"
  codex_jsonl="$(mktemp -t vip9r-orchestrator-codex.XXXXXX)"
  set +e
  npx -y @openai/codex exec \
    --json \
    -o "$final_message" \
    --sandbox "$sandbox" \
    -c "approval_policy=\"$approval_policy\"" \
    -C "$repo" \
    -- \
    "$prompt" \
    | "$codex_events" stream --reasoning "$codex_jsonl"
  pipeline_status=("${PIPESTATUS[@]}")
  status="${pipeline_status[0]}"
  renderer_status="${pipeline_status[1]:-0}"
  set -e

  if [[ "$status" -eq 0 && "$renderer_status" -ne 0 ]]; then
    status="$renderer_status"
  fi

  if [[ "$status" -ne 0 ]]; then
    echo "orchestrator-loop: codex exec failed with status $status" >&2
    rm -f "$final_message" "$codex_jsonl"
    exit "$status"
  fi

  cat "$final_message"
  last_line="$(awk 'NF { line = $0 } END { sub(/\r$/, "", line); print line }' "$final_message")"
  rm -f "$final_message" "$codex_jsonl"

  if [[ "$last_line" == "$signal" ]]; then
    echo "orchestrator-loop: continue signal received" >&2
    continue
  fi

  exit 0
done
