# codex-ps

`codex-ps` is a small terminal UI (TUI) that gives you a real-time overview of active Codex CLI sessions:

- working directory (shortened to `~/...`)
- git branch
- whether the session looks like it's working vs waiting for input
- subagent rollups (subagents are shown as a count on the parent session)

It reads session data from `~/.codex` (or `$CODEX_HOME`).

## Requirements

- Rust (stable) + Cargo
- `lsof` in `PATH` (used to discover active `codex` processes)
- A local Codex CLI install that writes sessions under `~/.codex` (or `$CODEX_HOME`)

## Install

Install from GitHub:

```bash
cargo install --git https://github.com/aelaguiz/codex-ps.git
```

Or install from a local checkout:

```bash
git clone https://github.com/aelaguiz/codex-ps.git
cd codex-ps
cargo install --path .
```

## Quickstart / Usage

Run the TUI:

```bash
codex-ps
# or, from the repo:
# cargo run --
```

Print a single JSON snapshot:

```bash
codex-ps --json | jq .
# or, from the repo:
# cargo run -- --json | jq .
```

Pick which hosts to aggregate (defaults to `local`):

```bash
codex-ps --host local
codex-ps --host all
codex-ps --host home,amirs-work-studio
```

## Remote aggregation (optional)

If `codex-ps` is installed on the remote host(s) (and your SSH aliases work), you can aggregate:

```bash
codex-ps --host all
```

Defaults include `home` and `amirs-work-studio` (override with `--host`).

## Development

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```
