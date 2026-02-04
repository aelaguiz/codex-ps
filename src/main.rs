mod app;
mod codex_home;
mod collector;
mod discovery;
mod git;
mod model;
mod rollout;
mod titles;
mod util;

use anyhow::Context;
use clap::Parser;
use std::io::Write;

use crate::codex_home::CodexHome;
use crate::collector::Collector;

const DEFAULT_REMOTE_HOSTS: &[&str] = &["home", "amirs-work-studio"];

#[derive(Debug, Parser)]
#[command(
    name = "codex-ps",
    version,
    about = "Real-time overview of active Codex CLI sessions"
)]
struct Cli {
    /// Output a single JSON snapshot (no TUI).
    #[arg(long)]
    json: bool,

    /// Host selector: local|home|amirs-work-studio|all, or a comma-list.
    #[arg(long, default_value = "local")]
    host: String,

    /// Override CODEX_HOME (default: $CODEX_HOME or ~/.codex).
    #[arg(long)]
    codex_home: Option<std::path::PathBuf>,

    /// Refresh interval for the TUI.
    #[arg(long, default_value_t = 1000)]
    refresh_ms: u64,

    /// SSH binary to use for remote aggregation (Phase 2).
    #[arg(long, default_value = "ssh")]
    ssh_bin: String,

    /// Remote `codex-ps` command (must be installed on the remote host).
    #[arg(long, default_value = "codex-ps")]
    remote_bin: String,

    /// SSH timeout per host.
    #[arg(long, default_value_t = 6000)]
    ssh_timeout_ms: u64,

    /// Include extra diagnostic fields in JSON / status line.
    #[arg(long)]
    debug: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let codex_home = CodexHome::resolve(cli.codex_home.clone())?;

    let hosts = parse_hosts(&cli.host)?;
    let mut collector = Collector::new(
        codex_home,
        cli.ssh_bin.clone(),
        cli.remote_bin.clone(),
        std::time::Duration::from_millis(cli.ssh_timeout_ms.max(100)),
    );

    if cli.json {
        let snapshot = collector.collect(&hosts, cli.debug)?;
        let out = serde_json::to_string_pretty(&snapshot).context("serialize JSON snapshot")?;
        let mut stdout = std::io::stdout();
        if let Err(e) = writeln!(stdout, "{out}") {
            // Common and harmless when piped to tools like `head`.
            if e.kind() != std::io::ErrorKind::BrokenPipe {
                return Err(e.into());
            }
        }
        return Ok(());
    }

    app::run_tui(collector, hosts, cli.refresh_ms, cli.debug)
}

fn parse_hosts(s: &str) -> anyhow::Result<Vec<String>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(vec!["local".into()]);
    }

    if s.eq_ignore_ascii_case("all") {
        let mut out = Vec::new();
        out.push("local".into());
        out.extend(DEFAULT_REMOTE_HOSTS.iter().map(|h| (*h).to_string()));
        return Ok(out);
    }

    let mut out: Vec<String> = Vec::new();
    for raw in s.split(',') {
        let h = raw.trim();
        if h.is_empty() {
            continue;
        }
        if !out.contains(&h.to_string()) {
            out.push(h.to_string());
        }
    }

    if out.is_empty() {
        out.push("local".into());
    }

    Ok(out)
}
