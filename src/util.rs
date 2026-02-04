use std::process::Stdio;
use std::process::{Command, Output};
use std::time::Duration;

use anyhow::Context;
use tempfile::NamedTempFile;
use wait_timeout::ChildExt;

pub fn run_cmd_with_timeout(mut cmd: Command, timeout: Duration) -> anyhow::Result<Output> {
    // Avoid deadlocks when commands emit a lot of output (pipes can fill and block the child).
    // We redirect stdout/stderr to temp files, then read them after completion.
    let stdout_file = NamedTempFile::new().context("create temp stdout file")?;
    let stderr_file = NamedTempFile::new().context("create temp stderr file")?;

    cmd.stdout(Stdio::from(
        stdout_file.reopen().context("reopen temp stdout file")?,
    ));
    cmd.stderr(Stdio::from(
        stderr_file.reopen().context("reopen temp stderr file")?,
    ));

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn command: {:?}", cmd))?;

    let status = child
        .wait_timeout(timeout)
        .with_context(|| format!("wait_timeout for {:?}", cmd))?;

    let status = match status {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("command timed out after {timeout:?}: {:?}", cmd);
        }
    };

    let stdout = std::fs::read(stdout_file.path())
        .with_context(|| format!("read temp stdout for {:?}", cmd))?;
    let stderr = std::fs::read(stderr_file.path())
        .with_context(|| format!("read temp stderr for {:?}", cmd))?;

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

pub fn system_time_to_unix_s(t: std::time::SystemTime) -> Option<i64> {
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
}

pub fn truncate_middle(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }

    let keep_left = (max - 1) / 2;
    let keep_right = max - 1 - keep_left;
    let left = &s[..keep_left.min(s.len())];
    let right = &s[s.len().saturating_sub(keep_right)..];
    format!("{left}…{right}")
}
