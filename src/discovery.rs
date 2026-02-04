use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::Context;
use once_cell::sync::Lazy;
use regex::Regex;

use crate::util::run_cmd_with_timeout;

static UUID_LIKE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
        .expect("uuid regex must compile")
});

#[derive(Clone, Debug)]
pub struct CodexLsofProcess {
    pub pid: i32,
    pub exe: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub tty: Option<String>,
    pub rollout_paths: Vec<PathBuf>,
}

/// Fastest robust SSOT we have on macOS: "active session" == a running `codex` process
/// that holds one or more rollout files open under `CODEX_HOME`.
///
/// Uses a single `lsof` call (instead of per-PID) to keep work bounded.
pub fn lsof_codex_processes(
    codex_home: &Path,
    timeout: Duration,
) -> anyhow::Result<Vec<CodexLsofProcess>> {
    let mut cmd = Command::new("lsof");
    cmd.args(["-n", "-P", "-c", "codex", "-F", "pfn"]);
    let output = run_cmd_with_timeout(cmd, timeout).context("lsof -c codex")?;

    if !output.status.success() {
        // On macOS, `lsof -c <name>` commonly returns exit code 1 when there are no matches.
        // That should mean "zero sessions", not "the collector is broken."
        if output.status.code() == Some(1) {
            return Ok(Vec::new());
        }
        anyhow::bail!("lsof failed with status {}", output.status);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut procs: Vec<CodexLsofProcess> = Vec::new();
    let mut current: Option<CodexLsofProcess> = None;
    let mut current_fd: Option<String> = None;

    for line in stdout.lines() {
        if let Some(pid_s) = line.strip_prefix('p') {
            if let Some(p) = current.take() {
                procs.push(p);
            }

            current_fd = None;
            let pid: i32 = match pid_s.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            current = Some(CodexLsofProcess {
                pid,
                exe: None,
                cwd: None,
                tty: None,
                rollout_paths: Vec::new(),
            });
            continue;
        }

        if let Some(fd) = line.strip_prefix('f') {
            current_fd = Some(fd.to_string());
            continue;
        }

        if let Some(name) = line.strip_prefix('n') {
            let Some(p) = current.as_mut() else { continue };
            let path = PathBuf::from(name);

            match current_fd.as_deref() {
                Some("cwd") => {
                    if p.cwd.is_none() {
                        p.cwd = Some(path.clone());
                    }
                }
                Some("txt") => {
                    if p.exe.is_none() {
                        p.exe = Some(path.clone());
                    }
                }
                Some("0") | Some("1") | Some("2") => {
                    if p.tty.is_none() && name.starts_with("/dev/tty") {
                        p.tty = Some(name.strip_prefix("/dev/").unwrap_or(name).to_string());
                    }
                }
                _ => {}
            }

            if name.contains("rollout-") && name.ends_with(".jsonl") && path.starts_with(codex_home)
            {
                p.rollout_paths.push(path);
            }
        }
    }

    if let Some(p) = current.take() {
        procs.push(p);
    }

    Ok(procs
        .into_iter()
        .filter(|p| !p.rollout_paths.is_empty())
        // Keep this tool scoped to CLI sessions; the Electron desktop app can hold
        // rollouts open for long periods, which is noisy and misleading for this dashboard.
        .filter(|p| {
            p.exe
                .as_ref()
                .is_none_or(|exe| !exe.to_string_lossy().contains("/Applications/Codex.app/"))
        })
        .collect())
}

pub fn extract_thread_id_from_rollout_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy();
    let stem = name.strip_suffix(".jsonl")?;
    if stem.len() < 36 {
        return None;
    }
    let candidate = &stem[stem.len() - 36..];
    let candidate = candidate.to_ascii_lowercase();
    if UUID_LIKE.is_match(&candidate) {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_thread_id_from_rollout_filename() {
        let p = PathBuf::from(
            "/Users/aelaguiz/.codex/sessions/2026/02/03/rollout-2026-02-03T16-12-22-019c2590-5605-7cd1-81b8-8a488af219a3.jsonl",
        );
        assert_eq!(
            extract_thread_id_from_rollout_path(&p).as_deref(),
            Some("019c2590-5605-7cd1-81b8-8a488af219a3")
        );
    }

    #[test]
    fn extract_thread_id_rejects_non_uuid_suffix() {
        let p = PathBuf::from("/tmp/rollout-2026-02-03T00-00-00-not-a-uuid.jsonl");
        assert!(extract_thread_id_from_rollout_path(&p).is_none());
    }
}
