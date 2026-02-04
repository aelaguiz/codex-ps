# Contributing

Thanks for helping improve `codex-ps`.

## Local checks

Before opening a PR, please run:

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

## Notes

- This tool is intentionally a **read-only observer**. Avoid changes that would mutate Codex sessions.
- Prefer small, bounded reads (rollout head/tail) over full-file parsing.
