---
title: "Codex Session Overview CLI — Worklog"
date: 2026-02-03
status: active
related:
  - docs/BUILD_REALTIME_TERMINAL_OVERVIEW_OF_RUNNING_CODEX_INSTANCES_2026-02-03.md
---

> Plan doc (SSOT): `docs/BUILD_REALTIME_TERMINAL_OVERVIEW_OF_RUNNING_CODEX_INSTANCES_2026-02-03.md`

## Phase 1 (Local MVP) Progress Update
- Work completed:
  - Created feature branch `codex-ps-mvp` (do not develop on `master`).
  - Scaffolded `codex-ps/` (Rust) with Ratatui TUI + `--json` snapshot mode.
  - Implemented local discovery via one `lsof -c codex -F pfn` pass + rollout `session_meta` parsing + title resolution from `~/.codex/.codex-global-state.json`.
  - Added a background collector thread so the TUI stays responsive even when `lsof` is slow.
  - Filtered out Codex desktop app sessions (keeps this focused on CLI).
- Tests run + results:
  - `cd codex-ps && cargo test` — ok (6 tests)
  - `cd codex-ps && cargo fmt --check` — ok
  - `cd codex-ps && cargo clippy -- -D warnings` — ok
- Issues / deviations:
  - Repo has no `origin` remote; branch was cut from local `master` HEAD.
  - Pinned `serde_json` to `=1.0.147` to avoid a `zmij` build break on the installed Rust toolchain.
  - `run_cmd_with_timeout` writes stdout/stderr to temp files to avoid pipe deadlocks with large `lsof` output.
- Next steps:
  - Phase 2: add `--host home|amirs-work-studio|all` via `ssh <alias> codex-ps --json` aggregation.

## Phase 2 (SSH Aggregation) Progress Update
- Work completed:
  - Implemented `--host all` and comma-list host selection (`local,home,amirs-work-studio` by default for `all`).
  - Added remote aggregation via `ssh <host> codex-ps --json` with per-host timeout and explicit `host_errors[]` reporting.
  - Updated TUI to show a HOST column and surface remote error count in the header.
  - Made `--json` output stable/fail-loud by emitting nulls for unknown fields (instead of omitting keys) and avoiding BrokenPipe panics when piping to `head`.
- Tests run + results:
  - `cd codex-ps && cargo test` — ok
  - `cd codex-ps && cargo fmt --check` — ok
  - `cd codex-ps && cargo clippy -- -D warnings` — ok
- Issues / deviations:
  - Remote hosts require `codex-ps` installed on-path; otherwise you’ll see `host_errors` like “command not found”.
- Next steps:
  - Phase 3: Kitty remote control actions (explicit + safe).

## Phase 1 (Local MVP) Progress Update
- Work completed:
  - Added subagent rollups: grouped TUI view shows one row per root thread id + `SUB` summary, instead of one row per spawned subagent.
  - Parsed subagent lineage from `session_meta.source` and propagated it into `--json` output fields for future UIs.
  - Made TID display stable/non-colliding (short UUID includes prefix + suffix, not just the first segment).
  - Surfaced `--debug` status reasons in the TUI via a `WHY` column (fail-loud diagnostics without leaving the dashboard).
  - Swapped `BRANCH`/`PWD` column order and shortened PWD display by stripping `$HOME` (`/Users/aelaguiz/...` → `~/...`).
  - Stopped eliding branch names (branch column is wider and no longer uses middle-truncation).
  - Fixed title resolver cache staleness when `.codex-global-state.json` disappears.
  - Centralized status thresholds as constants and added unit tests for classifier edge cases.
- Tests run + results:
  - `cd codex-ps && cargo test` — ok (14 tests)
  - `cd codex-ps && cargo fmt --check` — ok
  - `cd codex-ps && cargo clippy -- -D warnings` — ok
- Issues / deviations:
  - Manual QA still pending (non-blocking): validate grouped view and `SUB` counts against real multi-agent activity.
- Next steps:
  - Re-run remote manual validation on `home` + `amirs-work-studio` with real sessions.

## Phase 1 + Phase 2 (JSON Contract Polish) Progress Update
- Work completed:
  - Stabilized `--json` output for scripting: unknown optional lineage fields now emit `null` instead of omitting keys, and `host_errors[]` / `warnings[]` are always present (possibly empty).
  - Updated the plan doc’s Implementation Audit section to reflect current JSON contract and reviewer feedback dispositions.
- Tests run + results:
  - `cargo fmt --check` — ok
  - `cargo test` — ok
  - `cargo clippy -- -D warnings` — ok
- Issues / deviations:
  - Manual QA still pending (non-blocking): multi-session real-world validation and remote host reachability cases.
- Next steps:
  - Manual QA (local): 2–5 sessions, kill one mid-refresh, confirm no crash and rows update.
  - Manual QA (remote): `--host all` with one unreachable host; confirm `host_errors[]` behavior.
