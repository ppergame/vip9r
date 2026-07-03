# vip9r — orchestrator protocol

Session mechanics for the orchestrator role. requirements.md defines the roles
and durable decisions; this file is the runbook. Maintained by the user, like
requirements.md — suggest changes rather than editing.

## Session startup

- The user's request usually maps to a tracker milestone and may name a device
  target.
- Device prep:
  - `./scripts/vip9r-perf-daemon.py probe` list connected devices and their CPU
    topologies.
  - `./scripts/vip9r-perf-daemon.py prepare --serial <SERIAL>` sync runtime
    files onto the selected device.
  - `./scripts/vip9r-perf-daemon.py serve [--serial <SERIAL1> [--serial <SERIAL2>...]]`
    start the device queues. Device and CPU are selected for each request or
    grinder context.
- Host-only work: `./scripts/vip9r-perf-daemon.py serve`.
- `serve` blocks: run it in the background and keep it up for the session. This
  is the server allowing grinders to run wasm code.

## Tooling

- Hotspot attribution:
  `vip9r-perf-submit --target device profile --media MEDIA`; artifacts in
  temp/vip9r-profiles/, details in docs/d8.md."
- The grinder container doesn't have scripts/. If you want the grinder to test a
  spread of videos, give it the names or pass a one-off script via rust/

## Grinder runs

- Write a task file `temp/task-<slug>.md`.
- Spawn
  `nix run .#grinder -- run temp/task-<slug>.md --baseline WASM [--device INDEX:PIN]`
  in the background. The command prints progress updates until the implementor
  finishes, then prints the final message and a sandbox location like
  `temp/grinder.XXXXXX`.
  - Check up on a new job ~2 minutes after starting it to catch the first-turn
    failure below. After that, block on `TaskOutput` for the grinder job itself
    rather than polling with sleep timers. No need for line-by-line running
    commentary.
- Known failure: the grinder codex occasionally hangs or emits a malformed
  tool_search call on the first turn. Kill and retry, up to 3 times.
- Traces are copied to `temp/traces/` automatically; preserve that directory for
  auditing.

## Review and merge

- The sandbox `rust/` is a git repo with the pre-task tree tagged
  `orchestrator-base`.
- Review: `git -C temp/grinder.XXXXXX/rust diff orchestrator-base`
- Import from the project root:
  `git -C temp/grinder.XXXXXX/rust diff orchestrator-base | git apply --directory=rust`
- Verify with `jj diff`; do not trust a quiet apply.
- Task files and `temp/grinder.*` directories are transient; delete stale ones
  after merge.

## Session notes

- Keep transient handoff notes in `temp/orchestrator-notes.md` for the next
  orchestrator session or in case of context compaction. Notes are most useful
  at phase boundaries, not as running commentary.
