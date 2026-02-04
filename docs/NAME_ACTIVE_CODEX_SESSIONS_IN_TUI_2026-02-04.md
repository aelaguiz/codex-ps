---
title: "codex-ps — Name Active Sessions in TUI — Architecture Plan"
date: 2026-02-04
status: active
owners: [Amir]
reviewers: []
doc_type: architectural_change
related:
  - docs/BUILD_REALTIME_TERMINAL_OVERVIEW_OF_RUNNING_CODEX_INSTANCES_2026-02-03.md
  - /Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/rollout/session_index.rs
---

> Worklog: `docs/NAME_ACTIVE_CODEX_SESSIONS_IN_TUI_2026-02-04_WORKLOG.md`

# TL;DR

- **Outcome:** In the `codex-ps` TUI, Amir can select any active session row and assign a persistent human-friendly name that shows up in the dashboard (and `--json`) so it’s obvious what each instance is for.
- **Problem:** With many concurrent sessions, thread IDs + cwd/branch/title aren’t enough to remember intent; it’s easy to lose time hunting for “the right” instance.
- **Approach:** Add a small “session naming” layer keyed by `(host, thread_id)` with deterministic display precedence, stored in a `codex-ps`-owned state file (not in Codex rollouts), and a low-friction TUI rename flow.
- **Plan:** Phase 1 define the naming SSOT + persistence format + JSON shape → Phase 2 add TUI selection + rename/clear actions → Phase 3 (optional) remote rename/sync semantics.
- **Non-negotiables:**
  - Names must never be stored in or modify Codex rollouts (`rollout-*.jsonl` remains read-only).
  - Stable identity: naming is keyed by thread id (and host), not PID.
  - Fail-loud: if names can’t be loaded/saved, the UI still works and explains why in `--debug`.
  - Minimal UI friction: naming a session should take one obvious keybind + typing + Enter.
  - Named rows are always sorted above unnamed rows (scanability > recency).
  - No confusing precedence: when a custom name exists, it’s consistently shown; when it doesn’t, we fall back deterministically to existing title logic.

---

<!-- arch_skill:block:planning_passes:start -->
<!--
arch_skill:planning_passes
deep_dive_pass_1: done 2026-02-04
external_research_grounding: done 2026-02-04
deep_dive_pass_2: done 2026-02-04
recommended_flow: deep dive -> external research grounding -> deep dive again -> phase plan -> implement
note: This is a warn-first checklist only. It should not hard-block execution.
-->
<!-- arch_skill:block:planning_passes:end -->

---

# 0) Holistic North Star

## 0.1 The claim (falsifiable)
> If we add in-TUI, persistent session naming to `codex-ps` (select row → rename → name persists and displays), then Amir can reliably identify the intended purpose of each active Codex session in under 10 seconds when 5+ sessions are running, without opening other terminals, and without breaking the dashboard’s “truthiness” guarantees (thread id remains SSOT; rollouts remain read-only).

## 0.2 In scope
- UX surfaces (what users will see change):
  - A row can be selected in the TUI (keyboard navigation).
  - A new visible “name” display (either a new column or a deterministic override of the existing title column).
  - A rename action (e.g., `n` to set name, `x` to clear) with a small input modal/prompt inside the TUI.
  - Sort behavior: named rows are always shown above unnamed rows; within those groups, keep a stable deterministic order (e.g., activity desc).
  - `--debug` shows why a name is missing if persistence fails.
- Technical scope (what code will change):
  - Add a `codex-ps` state store for names keyed by `(host, thread_id)`.
  - Extend the `SessionRow`/snapshot model to carry an optional user name (for TUI + JSON).
  - Update the TUI renderer to support selection state + input capture for rename/clear.

## 0.3 Out of scope
- UX surfaces (what users must NOT see change):
  - No changes to Codex itself, its rollouts, or its behavior.
  - No “send input / focus terminals / kill sessions” actions (still Phase 3+ in the main plan).
- Technical scope (explicit exclusions):
  - No writing into Codex-owned files (`sessions/**/rollout-*.jsonl`, `archived_sessions/**`, etc.).
  - No requirement for cross-host name synchronization in v1 (remote hosts can have their own local names).

## 0.4 Definition of done (acceptance evidence)
- From the TUI:
  - You can select a session row and set a name in <10 seconds (no manual file editing).
  - The name persists across TUI restarts.
  - Clearing a name works and falls back to existing title behavior.
  - Named rows always appear above unnamed rows (even if an unnamed session is more recently active).
- From the data contract:
  - `codex-ps --json` includes an explicit `name` (or `display_name`) field per session row (null when missing).
  - Names are keyed by thread id (and host) and do not depend on PID stability.
- Evidence plan (common-sense; non-blocking):
  - Primary signal (minimal): manual QA checklist — start `codex-ps`, rename 2 sessions, restart `codex-ps`, confirm names persist and render correctly; clear one name and confirm fallback.
  - Optional second signal (if needed): unit tests for name store load/save + merge precedence — verify “latest write wins” and “custom name overrides fallback title”.
  - Default: do NOT add bespoke screenshot harnesses / drift scripts.
- Metrics / thresholds (if relevant):
  - Rename latency: <250ms UI stall budget for entering/leaving rename modal on a typical machine (subjective but noticeable).

## 0.5 Key invariants (fix immediately if violated)
- No silent fallbacks: persistence errors must be visible in `--debug` (and ideally a lightweight UI error line).
- No dual sources of truth: a session’s stable identity is thread id; PID is never used as a key.
- No Codex state mutation: we do not modify rollouts or any Codex-owned session metadata.
- Deterministic precedence: name resolution order is stable and documented (custom name → existing title resolution → explicit `unknown`).
- Deterministic sorting: named rows are always above unnamed rows (primary key is `has_name`).

---

# 1) Key Design Considerations (what matters most)

## 1.1 Priorities (ranked)
1) Low-friction UX (rename is obvious and fast)
2) Trustworthy persistence (no accidental wrong session names)
3) Scanability (named sessions are easy to spot; stable sort)

## 1.2 Constraints
- Correctness: naming must be keyed to stable session identity; collisions and host distinctions must be explicit.
- Performance: no heavy parsing per tick; name lookups must be O(1) and cached.
- Offline / latency: fully local; remote aggregation should continue to work.
- Compatibility / migration: adding a new JSON field must be backward-compatible for older remote binaries.
- Operational / observability: failures should be diagnosable (`--debug` reasons; minimal log spam).

## 1.3 Architectural principles (rules we will enforce)
- SSOT boundaries:
  - Session identity comes from rollout filename/thread id (existing contract).
  - Session name is owned by `codex-ps` and stored in a `codex-ps` state file.
- Keep collection pure:
  - Collector enriches rows with names (data), renderer only displays and collects input.
- Pattern propagation via comments (high leverage; no spam):
  - Add one crisp comment at the name-store boundary explaining key format, precedence, and safety (“rollouts are read-only”).

## 1.4 Known tradeoffs (explicit)
- Storage location:
  - Option A: `~/.config/codex-ps/session_names.jsonl` (preferred: clearly owned by `codex-ps`).
  - Option B: `CODEX_HOME/session_index.jsonl` (tempting for integration, but owned by Codex and contract riskier).
  - Chosen direction (draft): A for v1; revisit B only if we explicitly want names to propagate into Codex UIs.
- Scope of rename:
  - v1: local per-host names only (no remote rename).
  - later: remote rename via SSH action if we want it.

---

# 2) Problem Statement (existing architecture + why change)

## 2.1 What exists today
- `codex-ps` shows a real-time table keyed by thread id with columns like host, pid, state, age, title, branch, pwd.
- Titles are best-effort (Codex desktop global state, else cwd basename).
- There is no row selection and no action system in the TUI yet (only refresh/quit).
- `--json` is stable and fail-loud for scripting (nulls for unknown, arrays always present).

## 2.2 What’s broken / missing (concrete)
- Symptoms:
  - Multiple sessions in the same repo/worktree look identical.
  - Amir can’t encode “what I’m using this session for” in the dashboard.
  - You still end up opening terminals to remember intent.
- Root causes (hypotheses):
  - There’s no persistent, user-controlled label keyed to the stable session identity.
  - Current title sources (global state / cwd basename) are not intentionally descriptive.
- Why now:
  - Naming is the smallest UX addition that increases “glance trust” without needing full interactive controls.

## 2.3 Constraints implied by the problem
- Names must not accidentally drift to the wrong session (must be keyed to thread id, not row position or PID).
- UX must not add complexity that makes the dashboard slower or harder to scan.

---

<!-- arch_skill:block:research_grounding:start -->
# Research Grounding (external + internal “ground truth”)

## External anchors (papers, systems, prior art)
- `tmux rename-window` / iTerm session titles — adopt: lightweight, user-owned labels that persist and stay visible — applies because thread IDs + cwd/branch/title aren’t intent.
- `htop` / `top` — adopt: stable table, bounded per-tick work, never block the UI thread on slow I/O — applies because naming must not degrade refresh.
- XDG Base Directory Spec — adopt: store tool-owned state under `$XDG_CONFIG_HOME` (default `~/.config`) — applies because session names are `codex-ps`-owned, not Codex-owned.

## Internal ground truth (code as spec)
- Authoritative behavior anchors (do not reinvent):
  - `src/app.rs` — TUI refresh loop, worker thread boundary, grouping/sorting logic — evidence: `worker_loop(...)` and `group_sessions_for_display(...)` + stable sort.
  - `src/model.rs` — stable JSON contract (`null` for unknown optionals; arrays always present) — evidence: `SessionRow` serde config + `Snapshot` contract comment.
  - `src/titles.rs` — best-effort file enrichment with mtime caching and “clear cache if file disappears” — evidence: `refresh_if_changed()` + `clears_cache_when_global_state_disappears` test.
  - `src/codex_home.rs` — environment override + `dirs`-based path resolution pattern — evidence: `CodexHome::resolve(...)`.
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/rollout/session_index.rs` — append-only JSONL index with “latest entry wins” semantics and a bounded scan-from-end implementation — evidence: `append_thread_name(...)`, `scan_index_from_end(...)`, and tests like `find_thread_name_by_id_prefers_latest_entry`.
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/util.rs` — thread-name normalization contract (trim + reject empty) — evidence: `normalize_thread_name(...)` + `normalize_thread_name_trims_and_rejects_empty` test.
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/tui/src/chatwidget.rs` — prior art: in-TUI rename prompt + event handling in Codex itself — evidence: `show_rename_prompt()` and rename handling paths.
- Existing patterns to reuse:
  - `/Users/aelaguiz/workspace/openai-codex/codex-rs/core/src/rollout/session_index.rs` — “append-only JSONL + scan from end for newest match” — reuse for `codex-ps` name storage without rereading full files every tick.
  - `src/titles.rs` — “mtime cache; clear on disappear; fail-loud parse errors” — reuse for name-store refresh and diagnostics.
  - `src/model.rs` — “stable JSON keys” — reuse by adding `name: Option<String>` that serializes as `null` when missing.

## Open questions (evidence-based)
- Should `codex-ps` write into Codex’s `session_index.jsonl` (so names propagate into Codex UIs), or keep a `codex-ps`-owned store only? — evidence: confirm how Codex uses `session_index.jsonl` in practice and whether concurrent writers are safe/expected.
- Should the name display override the existing TITLE column or be a separate NAME column? — evidence: manual scan test with 10 sessions and see which is faster to identify at a glance.
- Remote aggregation semantics: should rename apply to remote hosts (via an explicit SSH action), or can we live with host-local naming stores in v1? — evidence: try `--host all` with one remote and validate what feels least surprising.
- Config path resolution: use `dirs::config_dir()` vs `XDG_CONFIG_HOME` fallback logic explicitly? — evidence: check `dirs` behavior on macOS and confirm where users expect tool state to live.
<!-- arch_skill:block:research_grounding:end -->

---

# 4) Current Architecture (as-is)

## 4.1 On-disk structure
```text
CODEX_HOME (default: ~/.codex)  [Codex-owned; read-only for codex-ps]
  sessions/YYYY/MM/DD/rollout-...-<thread_id>.jsonl   # session JSONL; first line is session_meta
  .codex-global-state.json                            # optional titles cache (thread_id -> title)

codex-ps (repo)
  src/main.rs         # CLI: `--json` vs TUI
  src/app.rs          # TUI loop + worker boundary + grouping/sorting + table renderer
  src/collector.rs    # collector SSOT: local discovery + remote SSH aggregation + enrichment
  src/discovery.rs    # `lsof` parsing + rollout path -> thread id extraction
  src/rollout.rs      # rollout parsing: session_meta + tail scan for pending function calls
  src/titles.rs       # global-state titles resolver (mtime-cached, best-effort)
  src/git.rs          # repo probing + branch lookup (bounded + cached)
  src/model.rs        # `Snapshot`/`SessionRow` JSON contract (nulls + arrays)
  src/util.rs         # time + bounded subprocess runner (`run_cmd_with_timeout`)
```

## 4.2 Control paths (runtime)

* Flow A: TUI tick → refresh → render
  * `app::run_tui(...)` spawns `worker_loop(...)` (collector thread) and runs `run_loop(...)` (UI thread)
  * UI tick calls `App::request_refresh()` → sends `WorkerCmd::Refresh` (non-blocking) (`src/app.rs`)
  * worker receives refresh → `Collector::collect(hosts, debug)` builds a `Snapshot` (`src/collector.rs`)
  * UI receives `WorkerMsg::Snapshot` → `group_sessions_for_display(...)` aggregates subagents and sorts rows (`src/app.rs`)
  * UI draws header + table (`header_line(...)`, `sessions_table(...)`, `row_for_session(...)`) (`src/app.rs`)

* Flow B: Local session discovery + enrichment (collector-internal)
  * `collect_local_rows(...)` calls `discovery::lsof_codex_processes(...)` as process SSOT (`src/discovery.rs`)
  * For each rollout path: `extract_thread_id_from_rollout_path(...)` yields stable id (`src/discovery.rs`)
  * `build_row(...)` enriches:
    * `rollout::read_session_meta(...)` (best-effort) (`src/rollout.rs`)
    * `titles::TitleResolver::get_title(...)` then cwd-basename fallback (`src/titles.rs`)
    * `git::GitCache::repo_root(...)` for repo_root/branch (cached) (`src/git.rs`)
    * `rollout::read_pending_function_call_from_tail(...)` as a status hint (`src/rollout.rs`)
    * `classify_status(...)` derives `SessionStatus` + debug reason (`src/collector.rs`)

* Flow C: Remote aggregation (Phase 2 behavior, already shipped)
  * `collect_remote_host(host, debug)` runs `ssh <host> codex-ps --json --host local` (`src/collector.rs`)
  * Parses remote `Snapshot` JSON → rewrites `row.host = <host>` locally (so the dashboard host label matches the SSH target)

* Flow D: Key handling (current)
  * `q` / Esc quits; `r` requests a refresh (`src/app.rs`)

## 4.3 Object model + key abstractions

* Key types:
  * `Snapshot` — root output for `--json` and the worker→UI message payload (`src/model.rs`)
  * `SessionRow` — one thread’s best-effort derived state, keyed by `thread_id` (`src/model.rs`)
  * `SessionStatus` — coarse state (`working|waiting|unknown`) derived from mtime + tail hints (`src/model.rs`, `src/collector.rs`)
  * `DisplaySessionRow` — UI-only grouping (root thread + subagent rollups) (`src/app.rs`)
* Ownership boundaries:
  * Codex owns `~/.codex/sessions/**` rollouts.
  * `codex-ps` is an observer (today); naming adds a small `codex-ps`-owned state file.

## 4.4 Observability + failure behavior today

* Logs: none (errors surface in UI and/or `--debug` fields).
* Failure surfaces:
  * Missing/unparseable metadata → shown as `unknown`, debug explains why.

## 4.5 UI surfaces (ASCII mockups, if UI work)

```ascii
Current `codex-ps` (today): no selection, no naming

codex-ps  hosts: local  sessions: 4  refresh: 1000ms  updated: 1s ago
Keys: r refresh   q quit

HOST  PID     TID            SUB  STATE AGE  TITLE     BRANCH     PWD
local 77998   019c2959…15bd0 0    WORK  12s  psmobile   fix/xp...  ~/ws/psmobile
local 53492   019c25d8…7b653 0    IDLE  4m   psmobile   main       ~/ws/psmobile
```

---

# 5) Target Architecture (to-be)

## 5.1 On-disk structure (future)

```text
~/.config/codex-ps/
  session_names.jsonl   # codex-ps-owned; append-only (latest wins), keyed by (host, thread_id)
```

## 5.2 Control paths (future)

* Flow A (new): Select + rename (non-blocking)
  * user moves selection (↑/↓) → presses `n` → rename modal captures text input (`src/app.rs`)
  * Enter commits: UI sends worker command `SetName { key, name }` (no file I/O on UI thread)
  * worker appends an update record to `session_names.jsonl`, refreshes/invalidates cache, and triggers a snapshot refresh
  * next snapshot overlays names → UI re-renders with named-first sort and updated NAME column

* Flow B (new): Select + clear
  * user selects row → presses `x` → UI sends `ClearName { key }`
  * worker appends a “clear” record (latest-wins) → next snapshot falls back to existing title behavior

## 5.3 Object model + abstractions (future)

* New types/modules:
  * `names::NamesStore` — mtime-cached append-only JSONL store with latest-wins semantics (`src/names.rs`)
  * `names::SessionNameKey { host, thread_id }` — stable identity for naming (host label as displayed + root thread id)
  * `App` selection state keyed by `SessionNameKey` (selection survives refresh/re-sort)
* Explicit contracts:
  * Append-only, latest-wins semantics (like other Codex JSONL indexes).
  * Name lookups are cached and refreshed on mtime change (no reread every tick).
  * Name input normalization is deterministic: trim whitespace; reject empty (treat as clear).
  * Display sorting is deterministic: `has_name` is the primary sort key (named rows always above unnamed).
* Public APIs (new/changed):
  * `NamesStore::get(key) -> Option<String>`
  * `NamesStore::set(key, name)` and `NamesStore::clear(key)`
  * `SessionRow.name: Option<String>` (serialized as `null` when missing; stable JSON schema)
  * Migration notes:
    * If the names file doesn’t exist, behavior is unchanged.
    * Remote aggregation remains backwards-compatible: older remotes omit `name` → deserialize as null.

## 5.4 Invariants and boundaries

* Fail-loud boundaries:
  * Persistence errors never crash the TUI; they show an error and keep rendering.
* Single source of truth:
  * Name SSOT is the names store; no parallel name sources in `codex-ps`.
  * Codex rollouts remain read-only and continue to define the session identity (thread id + host label).
* Performance / allocation boundaries:
  * Name store load is bounded (tail-only optional; cache in memory; avoid reread every tick).
* Determinism:
  * Named rows always sort above unnamed rows; within each group, sort remains stable and documented.
  * Name display is unquoted; empty is rendered as `(unset)` (UI), and `null` (JSON).

## 5.5 UI surfaces (ASCII mockups, if UI work)

```ascii
Target UX (naming + selection + named-first sort)

codex-ps  hosts: local,home  sessions: 7  refresh: 1000ms  updated: 1s ago
Sort: named-first, then activity desc   Keys: ↑/↓ select  n name  x clear  r refresh  q quit

HOST  PID     TID            SUB  STATE AGE  NAME                 TITLE     BRANCH     PWD
>home 22310   019c1abc…f00d  2    WORK  22s  backend triage       backend   main       ~/proj/foo
local 53492   019c25d8…7b653 0    IDLE  34m release triage       psmobile  main       ~/ws/psmobile
local 77998   019c2959…15bd0 0    WORK  3s   (unset)              psmobile  fix/xp...  ~/ws/psmobile
local 42230   019c2606…08c8c 1    UNK   11s  (unset)              codex     main       ~/ws/openai-codex

Status: (none)

Rename modal (press `n`):
+--------------------------------------------------------------------+
| Name session (home) 019c1abc…f00d                                  |
|                                                                    |
|  > backend triage_______________________________________________   |
|                                                                    |
|  Enter = Save      Esc = Cancel                                    |
+--------------------------------------------------------------------+

After save / clear / error:
Status: Saved name for (home) 019c1abc…f00d
Status: Cleared name for (home) 019c1abc…f00d
Status: ERROR: failed to save session name (see --debug)
```

---

# 6) Call-Site Audit (exhaustive change inventory)

## 6.1 Change map (table)

| Area | File | Symbol / Call site | Current behavior | Required change | Why | New API / contract | Tests impacted |
| ---- | ---- | ------------------ | ---------------- | --------------- | --- | ------------------ | -------------- |
| Model | `src/model.rs` | `SessionRow` | No user name field | Add `name: Option<String>` | Carry persisted name into UI/JSON | Stable JSON: null when missing | Update JSON snapshot tests (if any) |
| Store | `src/names.rs` (new) | `NamesStore` | N/A | Implement load/lookup/set/clear + mtime cache | Persist + resolve names | Append-only JSONL; latest wins | Unit tests for parsing + precedence |
| Collector | `src/collector.rs` | `Collector::collect(...)` | Returns rows without names | Overlay resolved name into each `SessionRow` | Keep UI/JSON dumb | `SessionRow.name` is resolved before leaving collector | Unit tests optional |
| Collector | `src/collector.rs` | `collect_remote_host(...)` | Rewrites `row.host = <ssh host>` | Ensure overlay uses rewritten host label for keying | Persist names per displayed host | Key = (host label, root thread id) | Manual QA w/ one remote |
| UI sort | `src/app.rs` | `group_sessions_for_display(...)` | Sorts by activity desc | Change to named-first primary key, then activity desc | Named scanability > recency | sort key: `has_name desc` then `last_activity` | Unit tests optional |
| UI select | `src/app.rs` | `run_loop(...)` | No selection | Add selection state + up/down navigation | Enables targeted rename/clear | Selection keyed by `SessionNameKey` | Manual QA |
| UI rename | `src/app.rs` | key handling + modal | No input capture | Add rename modal + text buffer + commit/cancel | Low-friction UX | `n` opens modal, Enter saves, Esc cancels | Manual QA |
| UI clear | `src/app.rs` | key handling | No actions besides refresh | Add `x` to clear selected row’s name | Quick undo | Clear appends null record | Manual QA |
| UI render | `src/app.rs` | `sessions_table(...)`, `row_for_session(...)` | No NAME column | Add NAME column + selection highlight | Make names visible | `(unset)` placeholder (unquoted) | Manual QA |
| CLI wiring | `src/main.rs` | module list | No names module | `mod names;` and construct `Collector` with a `NamesStore` | Make `--json` include names | stable contract | Unit tests optional |

## 6.2 Migration notes

* Deprecated APIs: none.
* Compatibility shims (if any): none (new optional field; older remotes ignore).
* Delete list (what must be removed): none (v1 adds new code paths; no deletions required).

## Pattern Consolidation Sweep (anti-blinders; scoped by plan)

| Area | File / Symbol | Pattern to adopt | Why (drift prevented) | Proposed scope (include/defer/exclude) |
| ---- | ------------- | ---------------- | ---------------------- | ------------------------------------- |
| Selection stability | `src/app.rs` (`App` state + refresh) | Key selection by stable `(host, thread_id)` instead of row index | Sorting/grouping changes every refresh; index-based selection would jump | include |
| Sorting SSOT | `src/app.rs` (`group_sessions_for_display`) | Centralize the “named-first” comparator | Prevent future regressions where collector sort and UI sort diverge | include |
| Persistence SSOT | `src/names.rs` (new) | “mtime-cached store; clear on missing; fail-loud parse errors” (like `src/titles.rs`) | Avoid subtle stale-cache bugs when file disappears or is edited | include |

---

# 7) Depth-First Phased Implementation Plan (authoritative)

> Rule: systematic build, foundational first; every phase has exit criteria + explicit verification plan (tests optional).

## Phase 1 — Naming SSOT + persistence

Status: COMPLETE (code + unit tests) — 2026-02-04

* Goal: reliably store and load names keyed by stable identity.
* Work:
  * Define name key semantics (`host + thread_id`).
  * Implement append-only `session_names.jsonl` with latest-wins semantics.
  * Extend `--json` rows with `name: null | string`.
* Verification (smallest signal):
  * Unit tests for load/merge + basic read/write.
* Exit criteria:
  * Names persist across runs; JSON output includes name field.
* Rollback:
  * Delete `~/.config/codex-ps/session_names.jsonl` (no other state).

## Phase 2 — TUI selection + rename/clear UX

Status: COMPLETE (code) — 2026-02-04

* Goal: rename sessions quickly without leaving the dashboard.
* Work:
  * Add row selection (keyboard navigation).
  * Add rename modal (text input) + clear action.
  * Add a stable display mapping (NAME column vs title override).
* Verification (smallest signal):
  * Manual QA checklist (rename 2 sessions, restart, confirm persistence).
* Exit criteria:
  * Rename/clear works and does not crash; errors are fail-loud in `--debug`.
* Rollback:
  * Feature-flag rename UI; keep read-only dashboard.

## Phase 3 — Optional remote semantics (defer)

Status: DEFERRED (v1 does not write to remote hosts)

* Goal: decide whether local TUI can rename remote sessions.
* Work:
  * If needed: add SSH action that writes name update on remote host.
* Verification (smallest signal):
  * Manual QA with one remote host.
* Exit criteria:
  * Remote renames are explicit and never block local refresh.
* Rollback:
  * Keep remote rename disabled by default.

---

# 8) Verification Strategy (common-sense; non-blocking)

## 8.1 Unit tests (contracts)
- Name store parsing and latest-wins semantics are unit-locked.
- Name precedence is unit-locked (custom name overrides fallback title).

## 8.2 Integration tests (flows)
- Not required for v1 (manual QA is sufficient); avoid building a harness.

## 8.3 E2E / device tests (realistic)
- N/A (terminal utility).

---

# 9) Rollout / Ops / Telemetry

## 9.1 Rollout plan
- Default: feature ships enabled; storage file is created only after first rename (no writes on read-only usage).

## 9.2 Telemetry changes
- None.

## 9.3 Operational runbook
- Debug checklist:
  - If names don’t show: run `codex-ps --debug` and confirm name-store load path and any parse error.

---

# 10) Decision Log (append-only)

## 2026-02-04 — Draft “name sessions in TUI” North Star

* Context: Amir wants a persistent, user-controlled label per active session inside the dashboard.
* Options:
  * Override title vs add a new NAME column
  * Store under `~/.config` vs under `CODEX_HOME` vs write into Codex’s `session_index.jsonl`
  * Sort by activity vs “named-first”
* Decision (draft): store names in a `codex-ps`-owned file under `~/.config/codex-ps/` and show as NAME (or title override) with deterministic precedence.
* Consequences: `codex-ps` becomes “read-only w.r.t Codex session state” rather than purely read-only.
* Follow-ups:
  - Confirm North Star + UX scope.
  - Do deep dive on existing `session_index.jsonl` semantics before deciding on any integration.
