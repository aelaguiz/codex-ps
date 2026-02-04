---
title: "Codex Session Overview CLI — Real-Time Instance Dashboard — Architecture Plan"
date: 2026-02-03
status: active
owners: [Amir]
reviewers: []
doc_type: new_system
related:
  - /Users/aelaguiz/workspace/openai-codex
---

> Worklog: `docs/BUILD_REALTIME_TERMINAL_OVERVIEW_OF_RUNNING_CODEX_INSTANCES_2026-02-03_WORKLOG.md`

# TL;DR

- **Outcome:** Running `codex-ps` shows a real-time, auto-updating overview of every active Codex CLI session (repo/worktree, branch, pwd, and “working vs waiting”) on local and configured SSH hosts.
- **Problem:** Amir often has multiple Codex instances spread across terminals/worktrees/hosts and lacks a single, glanceable source of truth for what each one is doing.
- **Approach:** Combine OS process discovery with Codex’s on-disk runtime metadata (under `~/.codex`) to reliably identify sessions and derive their current state, then render a tight terminal UI with an optional JSON output mode.
- **Plan:** Phase 1 local read-only dashboard → Phase 2 remote (SSH) discovery → Phase 3 interactive controls (Kitty remote control) and workflows.
- **Non-negotiables:**
  - Read-only and safe by default (no sending input/killing sessions in the MVP).
  - Fail-loud: the TUI uses `unknown` placeholders (and `--debug` explains why); JSON uses `null` for unknown optional fields and always includes `host_errors[]` / `warnings[]` (possibly empty) rather than silently dropping rows.
  - Fast refresh with bounded work per tick (no UI stalls; timeouts on slow probes).
  - Works with “messy reality” (sessions appearing/disappearing; partial metadata; non-git directories).
  - Extensible: collectors (data) and renderers (TUI/JSON) are decoupled for future controls.

---

<!-- arch_skill:block:implementation_audit:start -->
# Implementation Audit (authoritative)
Date: 2026-02-04
Verdict (code): COMPLETE
Manual QA: pending (non-blocking)

## Code blockers (why code isn’t done)
- None (code complete; see non-blocking follow-ups for manual QA).

## Reopened phases (false-complete fixes)
- None (Phase 1 code gaps were fixed on 2026-02-04).

## Missing items (code gaps; evidence-anchored; no tables)
- None (all Phase 1 “missing code” items addressed on 2026-02-04).

## Non-blocking follow-ups (manual QA / screenshots / human verification)
- Manual QA (local): run with 2–5 sessions, kill one mid-refresh, confirm no crash and row count updates.
- Manual QA (remote): ensure `codex-ps` exists on PATH for `home` and `amirs-work-studio`; run `--host all` with one host unreachable and confirm `host_errors[]` is shown.

## External second opinions
- Opus: received (2026-02-04)
  - Key points:
    - SSOT and boundaries look correct: thread id is primary; rollout parsing is centralized; grouping is done via `subagent_parent_thread_id`.
    - Diagnostics are present and useful in `--debug` (e.g., status reason via the `WHY` column).
    - JSON contract should be stable for scripting: prefer emitting `null` for unknown optional fields (and empty arrays for `host_errors[]`/`warnings[]`) over omitting keys.
    - Minor drift: `session_meta.id` mismatch is only surfaced in debug (not a top-level field).
  - Disposition: accepted — implemented “stable JSON nulls/arrays” and kept `meta_id_mismatch` as debug-only (non-blocking).
- Gemini: received (2026-02-04)
  - Key points:
    - Phase 1 + 2 implementation appears complete: grouping, lineage parsing, and SSH aggregation match the plan.
    - JSON schema nuance: make sure unknown optional fields are emitted as `null` (not missing keys) for easier scripting; TUI is fail-loud via `"unknown"` placeholders.
    - Local discovery timeout is bounded but hardcoded (10s) vs being a CLI-exposed knob.
  - Disposition: accepted — implemented stable JSON nulls/arrays; kept bounded-but-hardcoded local timeout as follow-up.
<!-- arch_skill:block:implementation_audit:end -->

<!-- arch_skill:block:planning_passes:start -->
<!--
arch_skill:planning_passes
deep_dive_pass_1: done 2026-02-03
external_research_grounding: done 2026-02-03
deep_dive_pass_2: done 2026-02-03
recommended_flow: deep dive -> external research grounding -> deep dive again -> phase plan -> implement
note: This is a warn-first checklist only. It should not hard-block execution.
-->
<!-- arch_skill:block:planning_passes:end -->

---

# 0) Holistic North Star

## 0.1 The claim (falsifiable)
> If we ship a `codex-ps` terminal utility that discovers all active Codex CLI sessions and displays an auto-updating table with (repo/worktree, branch, pwd, and activity state) derived from OS process info + Codex runtime metadata (`~/.codex`), then Amir can answer “what are all my Codex instances doing right now?” in under 10 seconds with high confidence, without opening other terminals, whenever there are 2+ concurrent sessions running.

## 0.2 In scope
- UX surfaces (what users will see change):
  - A single command (`codex-ps`) that opens a continuously refreshing overview (like a purpose-built `top`).
  - A stable `--json` output mode for scripting / future UI layers.
  - Optional host scoping (local first; remote hosts later): `--host local|home|amirs-work-studio|all`.
- Technical scope (what code will change):
  - Session discovery on a host: correlate “Codex processes” ↔ “Codex runtime metadata under `~/.codex`”.
  - Repo context derivation: cwd → git root/worktree → branch (best-effort, bounded).
  - State derivation: heuristics to classify “working / waiting for input / unknown” from available metadata (grounded in Codex source + observed `~/.codex` state).
  - Terminal rendering: a small, dependency-light TUI that updates efficiently.

## 0.3 Out of scope
- UX surfaces (what users must NOT see change):
  - No required changes to how Codex itself runs; this is an external observer tool.
  - No interactive “take over a session” in the MVP (no typing into sessions yet).
- Technical scope (explicit exclusions):
  - No mutation of Codex state files.
  - No process control in Phase 1 (no kill/stop/restart).
  - No Kitty remote control integration in Phase 1 (planned later).
  - No guarantee of perfect classification on day one; unknown/ambiguous states must be explicit.

## 0.4 Definition of done (acceptance evidence)
- On a machine with multiple running Codex instances, `codex-ps` shows one row per session with:
  - Host, PID (or stable session identifier), cwd/pwd, repo root/worktree, branch (if applicable), and a state label.
  - A “last update” and/or “last activity” timestamp to help spot stuck sessions.
- The view refreshes automatically and stays responsive even when some sessions disappear mid-refresh.
- Unknown fields are fail-loud (TUI shows `unknown`, JSON uses `null`) and do not crash the tool.
- Evidence plan (common-sense; non-blocking):
  - Primary signal (keep it minimal; prefer existing tests/checks): manual QA on a host with 2–5 concurrent Codex sessions — confirm the rows match reality by checking the underlying terminal tabs and `pwd`/branch.
  - Optional second signal (only if needed): compare discovered PIDs/paths against `ps`/`lsof` on the same host — confirm no phantom sessions.
- Metrics / thresholds (if relevant):
  - Refresh latency: target < 250ms typical on local host (kept snappy via bounded probes/timeouts; we don’t currently render per-tick timing in the UI).

## 0.5 Key invariants (fix immediately if violated)
- No silent fallbacks: if a value can’t be derived, emit `unknown` and keep rendering.
- Prefer live truth over stale files: if process is gone, the session should disappear quickly (MVP behavior; no explicit `stale` state).
- Bounded work per refresh: all probes must be time-limited to keep the UI snappy.
- Read-only by default: this tool must never write into `~/.codex` or send keystrokes in Phase 1.

---

# 1) Key Design Considerations (what matters most)

## 1.1 Priorities (ranked)
1) Accuracy + trustworthiness of the overview (no misleading “looks right” output)
2) Low cognitive load (glanceable; stable columns; consistent identifiers)
3) Extensibility for remote hosts and future interactive controls

## 1.2 Constraints
- Correctness: best-effort fields are fine, but ambiguity must be explicit.
- Performance: refresh must stay fast even with many sessions.
- Offline / latency: local must work offline; remote must degrade gracefully on slow links.
- Compatibility / migration: no changes required to Codex itself.
- Operational / observability: debug mode must explain why a field is unknown.

## 1.3 Architectural principles (rules we will enforce)
- Separate “collect” (discovery/probing) from “present” (TUI/JSON).
- Treat Codex source + on-disk runtime metadata as the contract; don’t invent semantics.
- Pattern propagation via comments (high leverage; no spam):
  - When we define the “session identity” and “state classification” contract, document it at the SSOT boundary module.

## 1.4 Known tradeoffs (explicit)
- “Working vs waiting” may only be heuristically detectable → we’ll bias toward `unknown` over incorrect certainty.
- Polling refresh is simplest for MVP → we’ll design for future event-driven improvements if needed.

---

# 2) Problem Statement (existing architecture + why change)

## 2.1 What exists today
- Amir runs multiple Codex CLI sessions concurrently (often in different repos/worktrees and sometimes over SSH).
- The “truth” of each session is scattered across terminal panes and whatever metadata Codex writes under `~/.codex`.
- There’s no single glanceable dashboard to answer: which sessions exist, where they are, and whether they’re blocked on input.

## 2.2 What’s broken / missing (concrete)
- Symptoms:
  - Time lost hunting through terminals to find “the right” Codex instance.
  - Uncertainty about whether a session is actively working vs waiting.
  - Hard to notice stuck/idle sessions and reclaim attention.
- Root causes (hypotheses):
  - No observer tool that correlates process + cwd/git context + Codex runtime state.
  - Terminal multiplexing (tabs/panes/SSH) hides global situational awareness.
- Why now:
  - The utility becomes a foundation for future interactive controls (Kitty remote control) and multi-host management.

## 2.3 Constraints implied by the problem
- The tool must tolerate partial info and still be useful.
- It must not require changes to Codex itself.
- It must support local first, with a clear path to SSH-host aggregation.

---

# 3) Research Grounding (external + internal “ground truth”)

<!-- arch_skill:block:research_grounding:start -->
## External anchors (papers, systems, prior art)
- `top` / `htop` / `ps` mental model — **adopt:** “stable identity + fast refresh + bounded probes” — **why it applies:** we need a trustworthy live dashboard, not a brittle report.
- `lsof` / procfs mental model — **adopt:** “process → open rollout file → thread id” — **why it applies:** Codex keeps the active rollout JSONL open; file path embeds the thread id.
- `watch(1)` / `btop` style refresh loops — **adopt:** fixed tick with stable sorting + incremental redraw — **why it applies:** the overview must stay glanceable and responsive.

## Internal ground truth (code as spec)
- Authoritative behavior anchors (do not reinvent):
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/config/mod.rs` — `find_codex_home()` resolves `CODEX_HOME` or defaults to `~/.codex` — foundation for locating all on-disk state.
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/rollout/mod.rs` — session storage subdirs: `CODEX_HOME/sessions` and `CODEX_HOME/archived_sessions`.
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/rollout/recorder.rs` — rollout file layout and naming:
    - `CODEX_HOME/sessions/YYYY/MM/DD/rollout-<YYYY-MM-DDThh-mm-ss>-<thread_id>.jsonl`
    - first JSONL record is always the `SessionMetaLine` (seed metadata), including optional `git` info.
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/protocol/src/protocol.rs` — wire schema for `SessionMetaLine` + session role/lineage:
    - `SessionMeta.source: SessionSource` distinguishes `cli` vs subagent sessions (`SessionSource::SubAgent(...)`).
    - `SubAgentSource::ThreadSpawn { parent_thread_id, depth }` is an explicit parent pointer for spawned subagents.
    - `SessionMeta.forked_from_id` exists for “forked thread” lineage (separate from subagents).
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/tools/handlers/collab.rs` — collab tool handler wires spawned subagents to the parent thread id:
    - `spawn_agent` sets `SessionSource::SubAgent(SubAgentSource::ThreadSpawn { parent_thread_id: session.conversation_id, depth })` for the child thread.
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/agent/guards.rs` — documents the spawn depth semantics:
    - `SubAgentSource::ThreadSpawn.depth` is “distance from root session” and is used to enforce a depth limit.
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/rollout/policy.rs` — persistence policy for rollouts:
    - many lifecycle/UI events (e.g., turn start/complete, request-user-input) are **not** persisted to rollouts
    - implication: “waiting for input” is not directly detectable from rollout events alone; we must supplement with process-level signals and/or other `~/.codex` state.
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/rollout/list.rs` — canonical helpers:
    - `read_session_meta_line(path)` expects the first record to be session meta and fails loudly otherwise.
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/rollout/session_index.rs` — optional `CODEX_HOME/session_index.jsonl` append-only mapping `{id, thread_name, updated_at}` (newest entry wins).
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/message_history.rs` — global history at `CODEX_HOME/history.jsonl`:
    - comment mentions legacy `conversation_id`, but current code writes `session_id` (thread id) + `ts` + `text` (so consumers should be tolerant).
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/shell_snapshot.rs` — per-thread shell snapshot files:
    - `CODEX_HOME/shell_snapshots/<thread_id>.sh` (or `.ps1`) with retention/cleanup.
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/state/src/runtime.rs` — optional `CODEX_HOME/state.sqlite` thread metadata DB (must be treated as optional; not present on this machine today).
  - `/Users/aelaguiz/workspace/openai-codex/codex-sessions.py` — existing “monitor active sessions” script:
    - valuable pattern: bounded parsing (head for `session_meta`, tail chunk for last events) + `--watch` refresh loop
    - caveat: its “input_wait” detection does not line up with the current rollout persistence policy, so that part should be treated as stale.

- On-disk ground truth (this machine; validate assumptions)
  - `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` exists and matches the above naming scheme.
    - First record: `type=session_meta` with payload including `id`, `cwd`, `originator`, `cli_version`, `source`, `model_provider`, and `git` (branch + commit hash when available).
    - Concrete subagent evidence (this is why `codex-ps` currently shows subagents as separate rows):
      - `/Users/aelaguiz/.codex/sessions/2026/02/03/rollout-2026-02-03T16-12-22-019c2590-5605-7cd1-81b8-8a488af219a3.jsonl` — `source: "cli"` (root session)
      - `/Users/aelaguiz/.codex/sessions/2026/02/03/rollout-2026-02-03T20-16-00-019c266f-631c-77c0-854f-2289c2d2fd8d.jsonl` — `source: {"subagent":{"thread_spawn":{"parent_thread_id":"019c2590-...","depth":1}}}` (spawned subagent thread)
  - `~/.codex/shell_snapshots/<thread_id>.sh` exists and is keyed by the same thread id as the rollout filename.
  - `~/.codex/.codex-global-state.json` contains a `thread-titles` mapping (thread id → human title) and other UI-ish hints (e.g., pinned thread ids, “terminal-open-by-key”), which we can optionally use to label rows.
  - `~/.codex/history.jsonl` exists and uses `{session_id, ts, text}` records (text-only, cross-session).

- Existing patterns to reuse
  - Extract identity + git context from the rollout **first line** (SessionMetaLine) instead of running `git`:
    - branch/commit are already collected once and persisted by Codex.
  - Use `SessionMeta.source` as the “role/lineage” SSOT:
    - treat `source=subagent.thread_spawn.parent_thread_id` as the canonical “child → parent” link.
    - group UI rows by “root thread id” and show `subagents: N` rather than N extra rows.
  - Use “head + tail bounded reads” for speed:
    - head: metadata (cwd, source, model_provider, git)
    - tail: lightweight “what happened recently” signals (last response item types, last timestamps)
  - Define “active session” as “a running process currently holds this rollout open”:
    - confirmed locally with `lsof` mapping `codex` PIDs ↔ `rollout-...<thread_id>.jsonl`
    - note: a single parent process may hold multiple rollouts open (subagents), so the primary key should be `thread_id`, not PID.

## Open questions (evidence-based)
- What is the minimal, cross-platform way to map “running Codex processes” → “open rollout file paths”?
  - Evidence to settle it:
    - macOS local: `lsof` clearly reveals the open rollout paths for `codex` processes.
    - Linux remote: verify we can rely on `lsof`, or else use `/proc/<pid>/fd` as the fallback.
- What is the correct way to collapse spawned subagents into a single top-level row?
  - Evidence to settle it:
    - `SessionMeta.source` in rollout head encodes `subagent.thread_spawn.parent_thread_id` (explicit parent pointer), so grouping is safe and does not require heuristics.
- How do we label “working vs waiting” with high trust, given rollouts do not persist turn lifecycle or request-user-input events?
  - Evidence to settle it:
    - process state/TTY signals (blocked read vs CPU-active),
    - last persisted `ResponseItem` types in rollout tail (`function_call`, `reasoning`, `message`, etc.) and time since last append,
    - optional UI state files when present (e.g., Codex desktop global state).
- How should `.codex/worktrees/...` be interpreted in the dashboard (real repo vs sandbox/worktree copy)?
  - Evidence to settle it:
    - correlate per-thread `cwd` + `git` info in SessionMetaLine with observed terminal processes launched at `.codex/worktrees/...`,
    - decide whether to show both “cwd” and an “origin workspace root” when derivable.
<!-- arch_skill:block:research_grounding:end -->

---

# 4) Current Architecture (as-is)

<!-- arch_skill:block:current_architecture:start -->
## 4.1 On-disk structure

The “ground truth” our tool will observe is produced by Codex itself (mostly Codex CLI in Rust) and written under `CODEX_HOME` (defaults to `~/.codex`; see `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/config/mod.rs`).

```text
CODEX_HOME (CODEX_HOME env, else ~/.codex)
  config.toml
  sessions/YYYY/MM/DD/
    rollout-<YYYY-MM-DDThh-mm-ss>-<thread_id>.jsonl     # primary persisted session log (JSONL)
  archived_sessions/
    rollout-<...>-<thread_id>.jsonl                      # archived rollouts
  history.jsonl                                          # global text-only prompt history (append-only JSONL)
  session_index.jsonl                                    # optional thread id <-> thread name (append-only JSONL)
  shell_snapshots/
    <thread_id>.sh                                       # optional: per-thread shell snapshot (3-day retention)
  log/
    codex-tui.log                                        # tracing log (can be very large)
  .codex-global-state.json                               # Codex desktop UI state (titles, pins, etc; optional)
  tmp/path/...                                           # arg0 helper symlinks (not session state)
  state.sqlite                                           # optional state DB (thread metadata); may not exist
```

Paths and formats are defined upstream by:
- `codex-rs/core/src/config/mod.rs` — `find_codex_home()` and `log_dir(cfg)` (`CODEX_HOME/log`).
- `codex-rs/core/src/rollout/recorder.rs` — `create_log_file()` and `rollout_writer()` (rollout naming and “session_meta is first record”).
- `codex-rs/protocol/src/protocol.rs` — `RolloutLine`, `RolloutItem`, `SessionMetaLine`, `TurnContextItem`, `EventMsg` (JSONL schema).
- `codex-rs/core/src/message_history.rs` — global `history.jsonl`.
- `codex-rs/core/src/rollout/session_index.rs` — optional `session_index.jsonl`.
- `codex-rs/core/src/shell_snapshot.rs` — `shell_snapshots/<thread_id>.{sh|ps1}`.
- `codex-rs/state/src/runtime.rs` — optional `CODEX_HOME/state.sqlite`.

## 4.2 Control paths (runtime)

* Flow A — Codex creates and appends rollout JSONL (session persistence):
  * `codex-rs/core/src/codex.rs` creates a `RolloutRecorder` for the session (`RolloutRecorder::new`).
  * `codex-rs/core/src/rollout/recorder.rs::create_log_file` creates `CODEX_HOME/sessions/YYYY/MM/DD/rollout-...-<thread_id>.jsonl`.
  * `codex-rs/core/src/rollout/recorder.rs::rollout_writer` writes the first record as `RolloutItem::SessionMeta(SessionMetaLine { meta, git })` and then appends subsequent persisted items (see `codex-rs/core/src/rollout/policy.rs` for what gets persisted).

* Flow B — Codex reads rollouts for resume/listing:
  * `codex-rs/tui/src/resume_picker.rs` uses `RolloutRecorder::list_threads` to build the resume picker list.
  * `codex-rs/core/src/rollout/list.rs::read_session_meta_line` reads the “head” of a rollout and expects the first record to be `SessionMetaLine` (fail-loud if not).

* Flow C — Global history and naming:
  * `codex-rs/core/src/message_history.rs::append_entry` appends user text to `CODEX_HOME/history.jsonl`.
  * `codex-rs/core/src/rollout/session_index.rs::append_thread_name` optionally appends thread name updates to `CODEX_HOME/session_index.jsonl` (append-only; newest wins).

* Flow D — Shell snapshot capture (optional feature):
  * `codex-rs/core/src/codex.rs` triggers `ShellSnapshot::start_snapshotting` (feature-gated).
  * `codex-rs/core/src/shell_snapshot.rs` writes `CODEX_HOME/shell_snapshots/<thread_id>.sh` (retention cleanup is internal).

* Flow E — “Which sessions are running right now?” (not a Codex API today):
  * Today this is mostly manual: `ps` to find `codex` processes and (on macOS) `lsof -p <pid>` to see which `rollout-...jsonl` files a process currently has open.
  * We validated locally that a live `codex` process holds its rollout file open, and that a single parent process may hold multiple rollout files open (subagents), so PID is not a stable session identifier.

## 4.3 Object model + key abstractions

* Key types (upstream; we must treat as contract):
  * `ThreadId` (UUID) — the stable session identifier (embedded in rollout filename).
  * `RolloutLine { timestamp, item }` — one JSON object per line in the rollout file (`codex-rs/protocol/src/protocol.rs`).
  * `SessionMetaLine { meta: SessionMeta, git?: GitInfo }` — always the first rollout record; includes `cwd`, `cli_version`, `source`, `model_provider`, and optional git branch/sha.
  * `TurnContextItem { cwd, approval_policy, sandbox_policy, model, ... }` — persisted in rollouts and can be used to derive “current cwd” (more accurate than session start cwd).
  * `SessionSource` — `cli|vscode|exec|mcp|subagent(...)|unknown` (important for grouping and display).

* Ownership boundaries:
  * Codex owns writing session state (`CODEX_HOME/**`).
  * `codex-ps` (our tool) is read-only: it must never mutate those files.

## 4.4 Observability + failure behavior today

* Logs:
  * Tracing logs are written to `~/.codex/log/codex-tui.log` by default (see `/Users/aelaguiz/workspace/openai-codex/docs/install.md`).
  * Rollout JSONL is effectively a per-session audit log (and also the resume source of truth).

* Failure surfaces / common failure modes:
  * Rollout header malformed → `read_session_meta_line` style parsing fails (must degrade gracefully in our observer).
  * `CODEX_HOME` overridden but missing/invalid → Codex fails to start (our tool should surface the resolved home).
  * “Waiting for input” is not directly recorded in rollouts:
    * `TurnStarted`, `TurnComplete`, `RequestUserInput`, etc. exist in protocol but are **not persisted** in rollouts (`codex-rs/core/src/rollout/policy.rs`).
    * implication: we must classify “working vs waiting” using process-level signals and/or rollout tail heuristics, and prefer `unknown` over guessing.

## 4.5 UI surfaces (ASCII mockups, if UI work)

Current reality is “no global dashboard”: you’re looking at scattered terminal panes/tabs plus optional log tailing. The closest built-in UI is the TUI resume picker (`codex-rs/tui/src/resume_picker.rs`), which lists sessions but does not answer “which are currently running”.
<!-- arch_skill:block:current_architecture:end -->

---

# 5) Target Architecture (to-be)

<!-- arch_skill:block:target_architecture:start -->
## 5.1 On-disk structure (future)

Implementation choice (MVP): **Rust + Ratatui (crossterm backend)**.

Why this is the default:
- It ships as a single fast binary (ideal for Phase 2: `ssh <host> codex-ps --json`).
- It supports a “pretty” modern TUI (tables/panels/colors/key hints) without fighting the framework.
- It keeps the architecture cleanly extensible: collectors → `SessionRow` SSOT → renderers (`tui` + `--json`) → later actions (Kitty remote control).
- It aligns with Codex’s own implementation language (`~/workspace/openai-codex/codex-rs/**`), so contracts and types map naturally.

Non-default alternative (kept as fallback, not MVP): **Python + Textual** for maximum UI polish, but heavier packaging/remote story.

Layout in this repo (current):
```text
codex-ps/
  Cargo.toml
  Cargo.lock
  README.md
  docs/
    BUILD_REALTIME_TERMINAL_OVERVIEW_OF_RUNNING_CODEX_INSTANCES_2026-02-03.md
    BUILD_REALTIME_TERMINAL_OVERVIEW_OF_RUNNING_CODEX_INSTANCES_2026-02-03_WORKLOG.md
  src/
    main.rs                                 # CLI args + orchestration
    app.rs                                  # Ratatui TUI renderer
    collector.rs                            # collect + enrich + classify + group
    codex_home.rs                           # CODEX_HOME resolution
    discovery.rs                             # active codex discovery (lsof)
    rollout.rs                              # bounded JSONL parsing (session_meta) + lineage
    titles.rs                               # title resolution + cache
    git.rs                                  # git probes (bounded + cached)
    model.rs                                # SessionRow SSOT + JSON schema
    util.rs                                 # small helpers
```

## 5.2 Control paths (future)

* Flow A (local; MVP):
  * Resolve `CODEX_HOME` (same rule as Codex: `CODEX_HOME` env else `~/.codex`).
  * Discover active `codex` CLI processes via a **single** `lsof -c codex -F pfn` pass:
    * per-PID `cwd` (OS truth, fd=`cwd`)
    * open `CODEX_HOME/**/rollout-*.jsonl` paths (thread id from filename suffix)
    * ignore Codex desktop app sessions (Electron) to keep this dashboard focused on CLI
  * Normalize to SessionId = `ThreadId` (from rollout filename or `session_meta.payload.id`).
  * For each active thread id:
    * Read the rollout **head** (first line) for `SessionMetaLine` (id, source/lineage, initial cwd, git branch/sha).
    * NOTE: rollout tail scanning is intentionally deferred for MVP; “current pwd” is derived from `lsof cwd` when available.
  * Optionally enrich rows:
    * thread title via `~/.codex/.codex-global-state.json` (MVP); `session_index.jsonl` fallback deferred
    * repo root via bounded `git -C <cwd> rev-parse --show-toplevel` (cache + timeout); branch is best-effort from `session_meta`
  * Render to TUI; repeat on tick.

* Flow B (`--json`):
  * Run a single pass of the same collectors; output stable JSON schema; exit.

* Flow C (remote aggregation; Phase 2):
  * For `--host home|amirs-work-studio|all`, run the same probe remotely.
  * Preferred implementation: `ssh <host> codex-ps --json` (requires tool installed remotely; simplest + most reliable).
  * Fallback implementation: `ssh <host> <probe-script>` that outputs JSON describing `pid -> open rollout paths` plus minimal rollout metadata (more fragile).

## 5.3 Object model + abstractions (future)

* New types/modules (our SSOT) — MVP reality (implemented):
  * `ThreadId` (string UUID) — SSOT identity; never use PID as the primary key.
  * `Snapshot` — top-level `--json` output (sessions + host_errors + warnings).
  * `SessionRow` — one row per thread id on one host (raw view; JSON-friendly).
  * `SessionStatus` — `working | unknown | waiting` (**no `stale` state** in MVP; sessions disappear when the process no longer holds a rollout open).
  * `SessionDebug` — optional diagnostics (only emitted when `--debug`).
  * `CodexLsofProcess` — raw process + fd view from `lsof` (pid, cwd, tty, open rollout paths).
  * Lineage fields live directly on `SessionRow`:
    * `session_source`, `forked_from_id`, `subagent_parent_thread_id`, `subagent_depth`.
  * Grouping is a TUI-only display layer:
    * One row per root thread id, with `SUB` summary (and optional W/U/WT breakdown in debug).
  * Deferred (explicitly not in MVP):
    * rollout tail scanning (`TurnContextItem.cwd`, last-item kinds) and any `RolloutTailMeta` helper.

* Explicit contracts:
  * **Collector contract:** collectors never panic on bad input; they return `unknown` plus reason.
  * **State contract:** “Working” means “recent rollout writes”; “Waiting” means “no rollout writes for a while”; “Unknown” is used conservatively when evidence is insufficient (and the reason is visible in `--debug`).
  * **Title contract:** a title is best-effort; resolver order is explicit and deterministic (global state → fallback to cwd basename → `unknown`). `session_index.jsonl` fallback is deferred.
  * **Grouping contract:** the default TUI view shows **one row per root thread id**; spawned subagents (per `SessionMeta.source.subagent.thread_spawn.parent_thread_id`) are collapsed into a `subagents=N` summary, with an optional “details” view in debug/Phase 3.

* Public APIs (new/changed):
  * `src/discovery.rs` — `lsof_codex_processes(codex_home, timeout) -> Vec<CodexLsofProcess>`
  * `src/discovery.rs` — `extract_thread_id_from_rollout_path(path) -> Option<ThreadId>`
  * `src/rollout.rs` — `read_session_meta(path) -> SessionMeta` (head-only)
  * `src/collector.rs` — `Collector::collect(hosts, debug) -> Snapshot`
  * `src/app.rs` — `run_tui(collector, hosts, refresh_ms, debug)`

## 5.4 Invariants and boundaries

* Fail-loud boundaries:
  * Never silently hide missing/unknown data; show `unknown` and keep rendering.
* Single source of truth:
  * `ThreadId` is the primary key; PID is always secondary display/diagnostic.
  * Rollout parsing is centralized in one module (no duplicated ad-hoc JSON parsing in renderers).
* Determinism contracts (time/randomness):
  * Any “active within N seconds” threshold is a constant with a single definition (centralized in the collector); debug output surfaces the observed “age” so the heuristic is interpretable.
* Performance / allocation boundaries:
  * Bounded read sizes for rollouts (head = 1 line; tail = <= 256–512KB).
  * Git probes are cached + time-bounded; never block the UI tick on slow repos.

## 5.5 UI surfaces (ASCII mockups, if UI work)

```ascii
codex-ps  live (refresh 1s)               view: grouped (root + subagents)    host: all
updated: 2s ago                           sort: activity desc                 filter: (none)

HOST   TID (stable)        SUB        TITLE                          BRANCH     PWD (current)            STATE  AGE   PID
local  019c2590…219a3      3 (2W/1U)  BUILD_CODEX_PS                  codex-ps   ~/workspace/utils        WORK   3s    91639
local  019c25d8…7b653      0          psmobile                        main      ~/workspace/psmobile     WAIT   4m    53492
local  019c2606…08c8c      1 (0W/1U)  openai-codex                     main      ~/workspace/openai-codex UNK    11s   42230
home   019c1abc…f00d       2 (0W/2W)  backend triage                  main      ~/proj/foo               WORK   22s   22310

Legend:
  SUB:  N (W/U/WT) = total subagents with status breakdown (optional)
  STATE: WORK / WAIT / UNK (UNK => see --debug / details)

Keys (later):  Enter expand row   d details   q quit   r refresh
```
<!-- arch_skill:block:target_architecture:end -->

---

# 6) Call-Site Audit (exhaustive change inventory)

<!-- arch_skill:block:call_site_audit:start -->
## 6.1 Change map (table)

| Area | File | Symbol / Call site | Current behavior | Required change | Why | New API / contract | Tests impacted |
| ---- | ---- | ------------------ | ---------------- | --------------- | --- | ------------------ | -------------- |
| Upstream session identity | `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/rollout/recorder.rs` | `create_log_file()` / `rollout_writer()` | Codex writes `rollout-...-<thread_id>.jsonl` and keeps it open while running | **No upstream change**; treat filename + first line as contract | Our observer needs a stable identity | `ThreadId` derived from rollout path and/or `session_meta.payload.id` | Unit tests for filename/thread id parsing |
| Subagent lineage + grouping | `/Users/aelaguiz/workspace/openai-codex/codex-rs/protocol/src/protocol.rs` + `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/tools/handlers/collab.rs` | `SessionMeta.source` / `SubAgentSource::ThreadSpawn{parent_thread_id,depth}` / collab `spawn_agent` | Spawned subagents are separate threads (separate rollouts) with an explicit parent pointer in session meta | Parse `source` from `session_meta` head and compute `root_thread_id`; group the TUI to show one row per root + `subagents=N` | Prevent row explosion; make the overview reflect “one session with helpers” | `SessionLineage { source, subagent_parent_thread_id?, depth? }` + `group_by_root(rows) -> group_rows` | Unit tests for source parsing (grouping exercised via TUI/manual QA) |
| Upstream JSONL schema | `/Users/aelaguiz/workspace/openai-codex/codex-rs/protocol/src/protocol.rs` | `SessionMetaLine` (first JSONL record) | Defines `type=session_meta` and payload shape | MVP: parse only `id`/`cwd`/`git` from the first line; tolerate huge first line | We only need a few fields for the dashboard; keep parsing bounded | `src/rollout.rs` — `read_session_meta(path) -> SessionMeta` | Unit tests for session_meta parsing |
| Persisted vs non-persisted events | `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/rollout/policy.rs` | `should_persist_event_msg` | Some lifecycle events are not persisted in rollouts | Classify state from what’s actually persisted (mtime heuristic) and fail-loud when ambiguous | Prevent false “waiting” certainty | `src/collector.rs` — `classify_status(...) -> SessionStatus` + `debug.status_reason` | Unit tests cover classifier thresholds + future-mtime skew |
| Thread title resolution | `~/.codex/.codex-global-state.json` | `thread-titles` map | Titles exist in global state (when available) | MVP: global-state titles → fallback to cwd basename → `unknown` (explicit source in debug); **defer** `session_index.jsonl` fallback | Make dashboard glanceable without blocking on extra files | `src/titles.rs` — `TitleResolver::get_title(thread_id)` | Unit tests cover resolver behavior (incl “stale cache” regression) |
| “Current pwd” (not just initial) | OS truth (`lsof`) + rollout head fallback | `lsof` `cwd` fd / `session_meta.cwd` | Current cwd is available via OS; rollouts contain only “start cwd” unless tail-scanned | MVP: use `lsof` cwd; fallback to `session_meta.cwd`; **defer** tail scan | Accurate “pwd” even after `cd`, without reading huge tail lines | `src/collector.rs` (cwd preference) | Unit tests for preference order are optional |
| Git context | `git` + rollout head | `git rev-parse --show-toplevel`, `SessionMeta.git` | Git branch/sha captured once at session start; repo root can be probed | Use session_meta for branch/sha; probe repo root with a short timeout and cache | Branch/worktree is a key column | `src/git.rs` — `GitCache::repo_root(...)` | Optional tests; keep manual QA as primary |
| Active session discovery | OS (`lsof`) | `lsof -c codex -F pfn` | No “list active sessions” API | Map PID → open rollout file(s) → thread id (SSOT) | Establish session existence + PID/TTY/cwd | `src/discovery.rs` — `lsof_codex_processes(...)` | Integration tests optional; unit tests cover filename parsing |
| Remote hosts | `ssh` | `ssh <alias> codex-ps --json` | Manual today | Phase 2: ask remote to run `codex-ps --json` and merge; timeouts + `host_errors[]` | Single dashboard across boxes | `src/collector.rs` — `collect_remote_host(...)` + `host_errors` | Manual verification (remote); local tests only |
| Renderer | `src/app.rs` | TUI refresh loop | N/A | Stable columns + background collector thread; refresh tick | Glanceable UI | `src/app.rs` — `run_tui(...)` | Manual QA; add debug/identity UX tests only if cheap |

## 6.2 Migration notes
- No changes are required to Codex itself; this is an external observer.
- `~/workspace/openai-codex/codex-sessions.py` overlaps with Phase 1 parsing/refresh behavior; we should treat it as a reference and avoid creating a second ad-hoc parser once `codex-ps` exists.

## 6.3 Pattern Consolidation Sweep (anti-blinders; scoped by plan)

| Area | File / Symbol | Pattern to adopt | Why (drift prevented) | Proposed scope (include/defer/exclude) |
| ---- | ------------- | ---------------- | ---------------------- | ------------------------------------- |
| Existing session monitor script | `/Users/aelaguiz/workspace/openai-codex/codex-sessions.py` | Use the same rollout parsing + state classification rules as `codex-ps` | Avoid multiple “truths” for what a session is / how state is inferred | defer |
| Upstream CLI | `/Users/aelaguiz/workspace/openai-codex/codex-rs/cli/src/main.rs` (new subcommand, hypothetical) | First-class “active sessions” command | Would be the most discoverable UX, but expands scope into Codex itself | exclude (for this project) |
| Title SSOT | `~/.codex/.codex-global-state.json` (MVP) + `session_index.jsonl` (future) | Deterministic precedence + explicit fallback order | Prevent “title flips” between sessions and keep UI stable | include |
<!-- arch_skill:block:call_site_audit:end -->

---

# 7) Depth-First Phased Implementation Plan (authoritative)

<!-- arch_skill:block:phase_plan:start -->
> Rule: systematic build, foundational first; every phase has exit criteria + explicit verification plan (tests optional). Prefer programmatic checks per phase; defer manual/UI verification to finalization. Avoid negative-value tests (deletion checks, visual constants, doc-driven gates). Also: document new patterns/gotchas in code comments at the canonical boundary (high leverage, not comment spam).

## Phase 1 — Local read-only overview (MVP)

* Status: done 2026-02-04
* Manual QA (non-blocking):
  * Still recommended: validate grouped view + subagent count and remote host behavior on real sessions.
* Goal:
  * Ship a fast, pretty local `codex-ps` TUI (Ratatui) plus `--json` output that correctly lists all *active* Codex threads (thread id SSOT) and their repo/pwd/state best-effort.
* Work:
  * Repo scaffolding:
    * Create `codex-ps/` Rust crate with a thin binary entrypoint and stable `--json` schema.
    * Add `ratatui` + `crossterm` (TUI), plus minimal deps for JSON parsing/formatting.
  * Core data model (SSOT):
    * `ThreadId` as primary key; PID only secondary.
    * `SessionRow` includes: host, thread id, title (best-effort), cwd (current), repo root (best-effort), branch (best-effort), state + age + pids.
    * `ThreadLineage` for grouping:
      * parse `SessionMeta.source` to detect `subagent.thread_spawn.parent_thread_id` and collapse child threads under the root session in the TUI.
  * Local collectors:
    * Process discovery + open-rollout mapping: a single macOS `lsof -c codex -F pfn` pass to get:
      * per-PID `cwd` (OS truth)
      * open `CODEX_HOME/**/rollout-*.jsonl` paths (thread id from filename suffix)
    * Rollout parsing:
      * head only: parse first JSON line (`session_meta`) for id/git/initial cwd (tolerate huge `base_instructions`).
      * NOTE: tail parsing is intentionally deferred; `turn_context` lines can be huge and aren’t needed when `lsof cwd` is available.
      * also parse `session_meta.source` (only for lineage/grouping; do not over-parse the full meta blob).
    * Title resolution: deterministic precedence `~/.codex/.codex-global-state.json` → cwd basename → `(unknown)`.
  * State classification (fail-loud):
    * `WORK` when there are recent rollout appends (mtime age <= 15s; tolerates tiny future skew).
    * `UNK` when evidence is insufficient (including a conservative “uncertain” window before committing to `WAIT`; reason is visible in `--debug`).
    * `WAIT` when the process is alive but there have been no rollout appends for a while (mtime age > 60s).
  * TUI:
    * Stable columns (HOST, PID, TID, SUB, STATE, AGE, TITLE, BRANCH, PWD), deterministic sort, refresh tick.
    * PWD is shortened for display by stripping `$HOME` (so `/Users/aelaguiz/...` becomes `~/...`).
    * Default view is grouped: one row per root thread id, with `SUB` showing spawned subagent count.
    * Collector runs in a background thread so the UI stays responsive even when `lsof` is slow.
  * Verification (smallest signal):
  * Programmatic:
    * `cargo test` (unit tests around parsing + classification + title resolver precedence).
    * `cargo fmt --check` and `cargo clippy` (keep it clean early).
    * `codex-ps --json` produces valid JSON; spot-check on a real session with a large `session_meta` first line (e.g., large `base_instructions`) to confirm it doesn’t crash.
  * Manual (short checklist, not a harness):
    * With 2–5 running Codex sessions, confirm row count and a couple of rows against `ps` + `lsof`.
    * Spawn a subagent (e.g. via `spawn_agent`) and confirm:
      * the table does **not** add a new top-level row; instead the parent row shows `SUB` incrementing.
    * Kill a session while `codex-ps` is running; confirm no crash and the row disappears on the next refresh (MVP behavior; no explicit `stale` state).
* Docs/comments (propagation; only if needed):
  * One SSOT comment in the `SessionStatus` / `classify_status` boundary explaining why “waiting” is heuristic (rollout persistence policy).
* Exit criteria:
  * TUI shows one row per active **root** thread id (not per PID); spawned subagents are summarized in `SUB`.
  * `--json` includes one row per active thread id (root + subagents), including lineage fields for grouping.
  * Shows: host, thread id, current cwd, git branch (at least from session meta when present), and a conservative state label.
  * Refresh stays responsive (no obvious UI stalls).
* Rollback:
  * Remove the binary; no state to migrate (read-only).

## Phase 2 — Remote host aggregation (SSH)

* Status: implemented 2026-02-03 (requires `codex-ps` installed on remote hosts)
* Goal:
  * `codex-ps --host all` shows a merged view of `local`, `home`, and `amirs-work-studio` without blocking the UI when a host is slow/unreachable.
* Work:
  * Preferred remote strategy: `ssh <alias> codex-ps --json` and aggregate locally.
  * Timeouts + fail-loud reporting:
    * Per-host SSH timeout (`--ssh-timeout-ms`).
    * Explicit `host_errors[]` in `--json` output (never silently drop a host).
  * CLI knobs:
    * `--ssh-bin` (defaults to `ssh`)
    * `--remote-bin` (defaults to `codex-ps`)
  * Remote prerequisite:
    * Install `codex-ps` on each host (any method is fine; e.g., `cargo install --path <repo>/codex-ps` or copying the built binary into PATH).
* Verification (smallest signal):
  * `codex-ps --host local --json` still works (no regression).
  * Manual: run with at least one live session on each host; spot-check with `ssh <host> ps` and/or `lsof`.
* Docs/comments (propagation; only if needed):
  * Document remote prereqs and fallbacks (e.g., `lsof` availability) in `--help`.
* Exit criteria:
  * A host outage never breaks local visibility; local remains useful.
* Rollback:
  * Remote mode remains opt-in; default is local.

## Phase 3 — Interactive controls (Kitty remote control)

* Goal:
  * Add explicit, safe actions (focus tab, copy thread id, open rollout file) without turning the observer into a “send input to sessions” tool by default.
* Work:
  * Action framework in the TUI (select row → action menu).
  * Kitty remote control integration for local host (future extension for remote is optional).
  * Keep dangerous actions out of MVP (no kill, no typing) unless explicitly added later.
* Verification (smallest signal):
  * Manual QA only (small number of actions; needs real terminals).
* Docs/comments (propagation; only if needed):
  * Safety contract: actions are always explicit and reversible; no automatic keystroke injection.
* Exit criteria:
  * Actions feel reliable and never fire without user intent.
* Rollback:
  * Feature-flag interactive mode; retain read-only observer as default.
<!-- arch_skill:block:phase_plan:end -->

---

# 8) Verification Strategy (common-sense; non-blocking)

> Principle: avoid verification bureaucracy. Prefer the smallest existing signal. If sim/video/screenshot capture is flaky or slow, rely on targeted instrumentation + a short manual QA checklist and keep moving.
> Default: 1–3 checks total. Do not invent new harnesses/frameworks/scripts unless they already exist in-repo and are the cheapest guardrail.
> Default: keep UI/manual verification as a finalization checklist (don’t gate implementation).
> Default: do NOT create “proof” tests that assert deletions, visual constants, or doc inventories. Prefer compile/typecheck + behavior-level assertions only when they buy confidence.
> Also: document any new tricky invariants/gotchas in code comments at the SSOT/contract boundary so future refactors don’t break the pattern.

## 8.1 Unit tests (contracts)
- `codex-ps` unit tests cover rollout filename parsing, `session_meta` parsing (including subagent lineage fields), title resolution, and state classification thresholds.

## 8.2 Integration tests (flows)
- Not added (kept lightweight; manual QA + JSON snapshot is the practical signal here).

## 8.3 E2E / device tests (realistic)
- N/A (terminal utility; manual QA checklist is sufficient for MVP).

---

# 9) Rollout / Ops / Telemetry

## 9.1 Rollout plan
- Not started.

## 9.2 Telemetry changes
- Not started.

## 9.3 Operational runbook
- Not started.

---

# 10) Decision Log (append-only)

## 2026-02-03 — Create plan doc + confirm North Star
- Context: Need a single SSOT architecture plan before doing code/research work.
- Options: N/A (doc creation).
- Decision: Drafted TL;DR + North Star; awaiting confirmation.
- Consequences: Next steps blocked on “yes/no” from Amir to ensure we’re building the right thing.
- Follow-ups:
  - Confirm North Star.
  - Do deep dive pass 1 (internal grounding in Codex source + `~/.codex`).

## 2026-02-03 — Ground in Codex source + local `~/.codex`
- Context: Need real contracts for session identity, on-disk state, and what “waiting” can mean.
- Options:
  - Build on rollouts + OS process mapping (chosen)
  - Try to infer everything from rollouts alone (rejected: key lifecycle events are not persisted)
- Decision:
  - Treat `ThreadId` (from rollout filename / `session_meta.payload.id`) as SSOT identity.
  - Define “active session” as “a running process holds the rollout file open” (validated via `lsof`).
  - Treat “working vs waiting” as heuristic and fail-loud when ambiguous.
- Consequences:
  - Target architecture is now grounded and ready for phased implementation without changing Codex itself.

## 2026-02-03 — Choose Rust + Ratatui for MVP TUI
- Context: We want an extensible “pretty” TUI that’s fast, ships as one binary, and won’t be painful for SSH aggregation.
- Options:
  - Rust + Ratatui (crossterm) (chosen)
  - Python + Textual (rejected for MVP: packaging/remote friction)
- Decision: Use Rust + Ratatui for `codex-ps` MVP; keep Python/Textual as a potential later experiment if we want a richer UI.
- Consequences: Phase 2 can prefer `ssh <host> codex-ps --json` as the simplest and most robust remote aggregation path.

## 2026-02-03 — Implement Phase 1 local MVP (`codex-ps`)
- Context: We need a trustworthy local dashboard before adding SSH aggregation or Kitty controls.
- Decision:
  - Implemented `codex-ps/` (Rust) with:
    - One-pass discovery via `lsof -c codex -F pfn`
    - Rollout `session_meta` parsing for git branch/commit
    - Title resolution via `~/.codex/.codex-global-state.json`
    - Responsive TUI (background collector) + stable `--json` snapshot output
- Evidence:
  - `cd codex-ps && cargo test` (6 tests) + `cargo fmt --check` clean.
- Follow-ups:
  - Phase 2: SSH aggregation (`ssh <alias> codex-ps --json`).

## 2026-02-03 — Implement Phase 2 SSH aggregation
- Context: North Star includes a single overview across local + SSH hosts.
- Decision:
  - Added `--host all` (and comma-lists) and a remote aggregation path: `ssh <host> codex-ps --json`.
  - Added `--ssh-bin`, `--remote-bin`, and `--ssh-timeout-ms` to make remote collection explicit and bounded.
  - Remote failures are fail-loud in `host_errors[]` (never silently dropped).
- Evidence:
  - `cd codex-ps && cargo test` + `cargo fmt --check` + `cargo clippy -- -D warnings` clean.
- Notes:
  - Remote hosts must have `codex-ps` installed on PATH (or set `--remote-bin`).

## 2026-02-04 — Implementation audit reopens Phase 1 gaps
- Context: We want to avoid “false complete” and make sure the dashboard is trustworthy (stable identity, fail-loud debug, no stale cached titles).
- Decision:
  - Inserted an authoritative Implementation Audit block (near the top) and reopened Phase 1 with concrete missing code items.
  - Treated manual QA as non-blocking follow-up evidence (important, but not “missing code”).
- Consequences:
  - Phase 1 is no longer considered complete until identity/debug/title-cache and classifier guardrails are fixed.
  - Phase 2 remains implemented (remote aggregation is code-complete, but still needs manual validation on real hosts).
- Follow-ups:
  - Implement the Phase 1 missing items (see the Implementation Audit block + Phase 1 section).

## 2026-02-04 — Subagent rollups: collapse spawned threads under root
- Context: `codex-ps` currently shows each spawned subagent as a top-level row, which makes the dashboard noisy when multi-agent workflows are active.
- Evidence (code as spec):
  - `SessionMeta.source` encodes subagent lineage:
    - `/Users/aelaguiz/workspace/openai-codex/codex-rs/protocol/src/protocol.rs`
  - `spawn_agent` sets `source=subagent.thread_spawn{parent_thread_id,depth}`:
    - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/tools/handlers/collab.rs`
  - On-disk example (this machine):
    - `/Users/aelaguiz/.codex/sessions/2026/02/03/rollout-2026-02-03T20-16-00-019c266f-631c-77c0-854f-2289c2d2fd8d.jsonl`
- Decision:
  - Treat `subagent.thread_spawn.parent_thread_id` as the SSOT “child → root” link for grouping in the **TUI default view**.
  - Show one row per root session + a `SUB` count (and optionally `SUB: W/U/W` breakdown in debug/expanded view later).
- Consequences:
  - Requires parsing `SessionMeta.source` from rollout head and adding a grouping layer above the raw `SessionRow`s.
  - JSON output can remain “raw per thread id” for now, but should include lineage fields so future UIs can group consistently.

## 2026-02-04 — Close Phase 1 reopen (grouped TUI + diagnostics)
- Context: Phase 1 was reopened by the audit due to real trust/UX gaps (subagent row explosion, misleading short TID, missing debug explanations, stale title cache, and untested classifier behavior).
- Decision:
  - Implemented subagent grouping in the TUI (root session row + `SUB` count; group status/age reflect the most-active child).
  - Parsed `session_meta.source` (and lineage) from rollout head and propagated it into `--json` rows.
  - Surfaced debug reasons in the TUI (`WHY` column when `--debug`).
  - Fixed `.codex-global-state.json` title cache staleness and added regression test coverage.
  - Centralized status thresholds as constants and added unit tests for `classify_status`.
- Evidence:
  - `cd codex-ps && cargo test` — ok
  - `cd codex-ps && cargo fmt --check` — ok
  - `cd codex-ps && cargo clippy -- -D warnings` — ok
- Follow-ups:
  - Manual QA (non-blocking): validate grouped view + `SUB` counts against real `spawn_agent` activity and ensure remote host behavior matches expectations.
