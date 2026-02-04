# Worklog — Name Active Sessions in TUI

Plan: `docs/NAME_ACTIVE_CODEX_SESSIONS_IN_TUI_2026-02-04.md`

## Phase 1 (Naming SSOT + persistence) Progress Update
- Work completed:
  - Added `src/names.rs` with an append-only JSONL `NamesStore` keyed by `(host, thread_id)` (`~/.config/codex-ps/session_names.jsonl` by default).
  - Extended `SessionRow` with `name: Option<String>` and wired `Collector` to overlay names into every collected row (local + remote).
- Tests run + results:
  - `cargo test -q` — pass
- Issues / deviations:
  - None.
- Next steps:
  - Add TUI selection + rename/clear UX and the named-first sort rule in the display grouping.

## Phase 2 (TUI selection + rename/clear UX) Progress Update
- Work completed:
  - Added row selection (↑/↓) with state keyed by `(host, thread_id)` so it survives refresh + re-sort.
  - Added rename modal (`n` opens, type, Enter saves, Esc cancels) with worker-thread persistence (no UI-thread file I/O).
  - Added clear action (`x`) and a NAME column; named sessions sort above unnamed.
- Tests run + results:
  - `cargo test -q` — pass
- Issues / deviations:
  - None.
- Next steps:
  - Final manual QA pass in a real terminal session (rename 2 rows, restart, confirm persistence; verify named-first order).

## External Review (read-only)
- Opus (anthropic/claude-opus-4.5):
  - Key points:
    - Naming SSOT/persistence approach looks correct (`src/names.rs`) and UI wiring matches the plan (`src/app.rs`).
    - Suggested adding a unit test to lock the “named rows always sort above unnamed” invariant.
  - Disposition:
    - Accepted: added `named_rows_sort_above_unnamed_rows` (`src/app.rs`).
- Gemini (gemini-3-pro-preview):
  - Key points:
    - Implementation is complete and idiomatic; highlights: named-first sort, host rewrite before keying, fail-loud error surfacing.
    - Notes: rename modal is intentionally minimal (no cursor navigation), selection scan is O(N) per render (fine for N<100).
  - Disposition:
    - Accepted: no code changes needed (notes match intended v1 scope).
