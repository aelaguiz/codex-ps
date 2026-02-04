# codex-ps

`codex-ps` is a small terminal UI that gives you a real-time overview of active Codex CLI sessions:

- working directory (shortened to `~/...`)
- git branch
- whether the session looks like it's working vs waiting for input
- subagent rollups (subagents are shown as a count on the parent session)

It reads session data from `~/.codex` (or `$CODEX_HOME`).

## Quickstart

Run the TUI:

```bash
cargo run --
```

Print a single JSON snapshot:

```bash
cargo run -- --json | jq .
```

Install locally:

```bash
cargo install --path .
```

## Remote aggregation (optional)

If `codex-ps` is installed on the remote host(s), you can aggregate:

```bash
codex-ps --host all
```

Defaults include `home` and `amirs-work-studio` (override with `--host`).
