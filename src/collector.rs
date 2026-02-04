use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use anyhow::Context;

use crate::codex_home::CodexHome;
use crate::discovery::{extract_thread_id_from_rollout_path, lsof_codex_processes};
use crate::git::GitCache;
use crate::model::{HostError, SessionBuilder, SessionDebug, SessionRow, SessionStatus, Snapshot};
use crate::names::{NamesStore, SessionNameKey};
use crate::rollout::{
    PendingFunctionCall, read_pending_function_call_from_tail, read_session_meta,
};
use crate::titles::TitleResolver;
use crate::util::{system_time_to_unix_s, truncate_middle};

const STATUS_WORKING_MAX_AGE_SECS: u64 = 15;
const STATUS_UNCERTAIN_MAX_AGE_SECS: u64 = 60;
const STATUS_MAX_FUTURE_MTIME_SKEW_SECS: u64 = 2;
const ROLLOUT_TAIL_MAX_BYTES: u64 = 512 * 1024;

pub struct Collector {
    codex_home: CodexHome,
    titles: TitleResolver,
    names: NamesStore,
    git_cache: GitCache,
    ssh_bin: String,
    remote_bin: String,
    ssh_timeout: Duration,
    rollout_tail_cache: HashMap<std::path::PathBuf, TailCacheEntry>,
}

#[derive(Clone, Debug)]
struct TailCacheEntry {
    mtime: Option<SystemTime>,
    parsed_for_mtime: bool,
    pending_call: Option<PendingFunctionCall>,
}

impl Collector {
    pub fn new(
        codex_home: CodexHome,
        ssh_bin: String,
        remote_bin: String,
        ssh_timeout: Duration,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            titles: TitleResolver::new(&codex_home.root),
            names: NamesStore::new()?,
            git_cache: GitCache::new(Duration::from_secs(5)),
            codex_home,
            ssh_bin,
            remote_bin,
            ssh_timeout,
            rollout_tail_cache: HashMap::new(),
        })
    }

    pub fn collect(&mut self, hosts: &[String], debug: bool) -> anyhow::Result<Snapshot> {
        // Always include at least local.
        let mut host_list = hosts.to_vec();
        if host_list.is_empty() {
            host_list.push("local".into());
        }

        let mut warnings: Vec<String> = Vec::new();
        let mut host_errors: Vec<HostError> = Vec::new();
        let mut sessions: Vec<SessionRow> = Vec::new();

        if host_list.iter().any(|h| h == "local") {
            match self.collect_local_rows(debug) {
                Ok((mut rows, mut local_warnings)) => {
                    sessions.append(&mut rows);
                    warnings.append(&mut local_warnings);
                }
                Err(e) => host_errors.push(HostError {
                    host: "local".into(),
                    error: format!("{e}"),
                }),
            }
        }

        for host in host_list.iter().filter(|h| *h != "local") {
            match self.collect_remote_host(host, debug) {
                Ok(mut snap) => {
                    for row in &mut snap.sessions {
                        row.host = host.clone();
                    }
                    sessions.extend(snap.sessions);
                    if let Some(mut w) = snap.warnings.take() {
                        warnings.append(&mut w);
                    }
                    if let Some(mut he) = snap.host_errors.take() {
                        host_errors.append(&mut he);
                    }
                }
                Err(e) => host_errors.push(HostError {
                    host: host.clone(),
                    error: format!("{e}"),
                }),
            }
        }

        if let Err(e) = self.names.refresh_if_changed() {
            if debug {
                warnings.push(format!(
                    "names store ({}): {e}",
                    self.names.path().display()
                ));
            }
        }
        for row in &mut sessions {
            let key = SessionNameKey {
                host: row.host.clone(),
                thread_id: row.thread_id.clone(),
            };
            row.name = self.names.get_cached(&key).map(|s| s.to_string());
        }

        let now = SystemTime::now();
        sessions.sort_by(|a, b| {
            let a_ts = a.last_activity_unix_s.unwrap_or(i64::MIN);
            let b_ts = b.last_activity_unix_s.unwrap_or(i64::MIN);
            b_ts.cmp(&a_ts)
                .then_with(|| a.host.cmp(&b.host))
                .then_with(|| a.thread_id.cmp(&b.thread_id))
        });

        Ok(Snapshot {
            generated_at_unix_s: system_time_to_unix_s(now).unwrap_or(0),
            host: host_list.join(","),
            sessions,
            host_errors: Some(host_errors),
            warnings: Some(warnings),
        })
    }

    pub fn set_session_name(
        &mut self,
        key: SessionNameKey,
        name: String,
    ) -> anyhow::Result<Option<String>> {
        self.names.set(key, name)
    }

    pub fn clear_session_name(&mut self, key: SessionNameKey) -> anyhow::Result<()> {
        self.names.clear(key)
    }

    fn collect_local_rows(
        &mut self,
        debug: bool,
    ) -> anyhow::Result<(Vec<SessionRow>, Vec<String>)> {
        // Single `lsof` call for all `codex` processes. This is the most reliable and
        // least error-prone SSOT for "what is actively running right now?"
        let lsof_procs = lsof_codex_processes(&self.codex_home.root, Duration::from_secs(10))?;
        let now = SystemTime::now();

        let mut warnings: Vec<String> = Vec::new();
        let mut by_thread: HashMap<String, SessionBuilder> = HashMap::new();

        for p in lsof_procs {
            for rollout_path in p.rollout_paths {
                let Some(thread_id) = extract_thread_id_from_rollout_path(&rollout_path) else {
                    if debug {
                        warnings.push(format!(
                            "unparseable rollout filename: {}",
                            rollout_path.display()
                        ));
                    }
                    continue;
                };

                let entry = by_thread
                    .entry(thread_id.clone())
                    .or_insert_with(|| SessionBuilder {
                        thread_id: thread_id.clone(),
                        pids: Vec::new(),
                        tty: p.tty.clone(),
                        proc_cwd: p.cwd.clone(),
                        rollout_path: Some(rollout_path.clone()),
                        proc_command_sample: p
                            .exe
                            .as_ref()
                            .map(|x| x.to_string_lossy().to_string())
                            .or_else(|| Some("codex".into())),
                    });

                if !entry.pids.contains(&p.pid) {
                    entry.pids.push(p.pid);
                }

                // Prefer the newest rollout path (in case something moved between dirs).
                entry.rollout_path = Some(rollout_path.clone());

                if entry.proc_cwd.is_none() {
                    entry.proc_cwd = p.cwd.clone();
                }
                if entry.tty.is_none() {
                    entry.tty = p.tty.clone();
                }
                if entry.proc_command_sample.is_none() {
                    entry.proc_command_sample = p
                        .exe
                        .as_ref()
                        .map(|x| x.to_string_lossy().to_string())
                        .or_else(|| Some("codex".into()));
                }
            }
        }

        let mut sessions: Vec<SessionRow> = by_thread
            .into_values()
            .map(|b| self.build_row(b, now, debug))
            .collect();

        sessions.sort_by(|a, b| {
            let a_ts = a.last_activity_unix_s.unwrap_or(i64::MIN);
            let b_ts = b.last_activity_unix_s.unwrap_or(i64::MIN);
            b_ts.cmp(&a_ts).then_with(|| a.thread_id.cmp(&b.thread_id))
        });

        Ok((sessions, warnings))
    }

    fn build_row(&mut self, b: SessionBuilder, now: SystemTime, debug: bool) -> SessionRow {
        let mut row = SessionRow {
            host: "local".into(),
            thread_id: b.thread_id.clone(),
            pids: b.pids.clone(),
            tty: b.tty.clone(),
            title: None,
            name: None,
            cwd: None,
            repo_root: None,
            git_branch: None,
            git_commit: None,
            session_source: None,
            forked_from_id: None,
            subagent_parent_thread_id: None,
            subagent_depth: None,
            status: SessionStatus::Unknown,
            last_activity_unix_s: None,
            rollout_path: b
                .rollout_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            debug: None,
        };

        let mut dbg = SessionDebug {
            status_reason: None,
            process_command_sample: b
                .proc_command_sample
                .as_ref()
                .map(|s| truncate_middle(s, 120)),
            proc_cwd_source: None,
            meta_parse_error: None,
            meta_id_mismatch: None,
            repo_probe_error: None,
            title_source: None,
        };

        // CWD preference:
        // 1) OS truth: lsof cwd
        // 2) session_meta.cwd (if parseable)
        if let Some(cwd) = b.proc_cwd.as_ref() {
            row.cwd = Some(cwd.to_string_lossy().to_string());
            dbg.proc_cwd_source = Some("lsof".into());
        }

        // Rollout metadata (best-effort).
        let meta = match b.rollout_path.as_ref() {
            Some(p) => match read_session_meta(p) {
                Ok(m) => Some(m),
                Err(e) => {
                    dbg.meta_parse_error = Some(format!("{e}"));
                    None
                }
            },
            None => None,
        };

        if row.cwd.is_none() {
            if let Some(m) = meta.as_ref().and_then(|m| m.cwd.clone()) {
                row.cwd = Some(m);
                dbg.proc_cwd_source = Some("session_meta".into());
            }
        }

        if let Some(meta) = meta {
            if let Some(id) = meta.id.as_ref() {
                if id != &row.thread_id {
                    dbg.meta_id_mismatch =
                        Some(format!("meta.id={id} != filename.id={}", row.thread_id));
                }
            }
            row.git_branch = meta.git_branch;
            row.git_commit = meta.git_commit;
            row.session_source = meta.session_source;
            row.forked_from_id = meta.forked_from_id;
            row.subagent_parent_thread_id = meta.subagent_parent_thread_id;
            row.subagent_depth = meta.subagent_depth;
        }

        // Title (best-effort): global state titles → fallback to last path segment of cwd.
        if let Ok(Some((t, src))) = self.titles.get_title(&row.thread_id) {
            row.title = Some(t);
            dbg.title_source = Some(src.into());
        } else if let Some(cwd) = row.cwd.as_ref() {
            row.title = cwd
                .rsplit(std::path::MAIN_SEPARATOR)
                .next()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            if row.title.is_some() {
                dbg.title_source = Some("cwd_basename".into());
            }
        }

        // Repo root (best-effort, cached).
        if let Some(cwd_s) = row.cwd.as_ref() {
            let cwd = std::path::Path::new(cwd_s);
            let (root, err) = self
                .git_cache
                .repo_root(cwd, Duration::from_millis(250))
                .unwrap_or((None, Some("git probe error".into())));
            row.repo_root = root.map(|p| p.to_string_lossy().to_string());
            dbg.repo_probe_error = err;
        }

        // Last activity: rollout mtime when available.
        let mut last_activity: Option<SystemTime> = None;
        if let Some(p) = b.rollout_path.as_ref() {
            if let Ok(m) = std::fs::metadata(p) {
                last_activity = m.modified().ok();
            }
        }
        row.last_activity_unix_s = last_activity.and_then(system_time_to_unix_s);

        let pending_call = b
            .rollout_path
            .as_ref()
            .and_then(|p| self.pending_function_call_hint(p.as_path(), last_activity, &mut dbg));

        row.status = classify_status(now, last_activity, pending_call.as_ref(), &mut dbg);

        if debug {
            row.debug = Some(dbg);
        }

        row
    }

    fn collect_remote_host(&self, host: &str, debug: bool) -> anyhow::Result<Snapshot> {
        // Phase 2 strategy: ask the remote machine to run `codex-ps --json` and aggregate.
        // This keeps parsing/state logic identical on every host.
        let mut cmd = std::process::Command::new(&self.ssh_bin);
        cmd.args(["-o", "BatchMode=yes"]);
        cmd.args(["-o", "ConnectTimeout=3"]);
        cmd.arg(host);
        cmd.arg(&self.remote_bin);
        cmd.arg("--json");
        cmd.arg("--host");
        cmd.arg("local");
        if debug {
            cmd.arg("--debug");
        }

        let out = crate::util::run_cmd_with_timeout(cmd, self.ssh_timeout)
            .with_context(|| format!("ssh {host} {} --json", self.remote_bin))?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!(
                "ssh {host} failed (status {}): {}",
                out.status,
                truncate_middle(stderr.trim(), 200)
            );
        }

        let snap: Snapshot = serde_json::from_slice(&out.stdout)
            .with_context(|| format!("parse remote JSON snapshot from host={host}"))?;
        Ok(snap)
    }

    fn pending_function_call_hint(
        &mut self,
        rollout_path: &std::path::Path,
        mtime: Option<SystemTime>,
        dbg: &mut SessionDebug,
    ) -> Option<PendingFunctionCall> {
        let entry = self
            .rollout_tail_cache
            .entry(rollout_path.to_path_buf())
            .or_insert_with(|| TailCacheEntry {
                mtime: None,
                parsed_for_mtime: false,
                pending_call: None,
            });

        if entry.mtime != mtime {
            entry.mtime = mtime;
            entry.parsed_for_mtime = false;
            entry.pending_call = None;
            return None;
        }

        if !entry.parsed_for_mtime {
            entry.parsed_for_mtime = true;
            entry.pending_call =
                match read_pending_function_call_from_tail(rollout_path, ROLLOUT_TAIL_MAX_BYTES) {
                    Ok(v) => v,
                    Err(e) => {
                        // Tail parsing is best-effort; fall back to mtime heuristics.
                        dbg.status_reason = Some(format!("tail parse failed: {e}"));
                        None
                    }
                };
        }

        entry.pending_call.clone()
    }
}

fn classify_status(
    now: SystemTime,
    last_activity: Option<SystemTime>,
    pending_call: Option<&PendingFunctionCall>,
    dbg: &mut SessionDebug,
) -> SessionStatus {
    if let Some(call) = pending_call {
        if call.name == "request_user_input" {
            dbg.status_reason = Some(format!("waiting for user input (call_id={})", call.call_id));
            return SessionStatus::Waiting;
        }

        dbg.status_reason = Some(format!(
            "pending tool call: {} (call_id={})",
            call.name, call.call_id
        ));
        return SessionStatus::Working;
    }

    // If we can't even get last activity, stay unknown (fail-loud).
    let Some(ts) = last_activity else {
        dbg.status_reason = Some("no rollout mtime".into());
        return SessionStatus::Unknown;
    };

    // `now` is captured before we stat the file, so a tiny skew is normal. Treat small
    // "future" mtimes as "just now" instead of flipping to Unknown.
    let age = match now.duration_since(ts) {
        Ok(d) => d,
        Err(_) => match ts.duration_since(now) {
            Ok(skew) if skew <= Duration::from_secs(STATUS_MAX_FUTURE_MTIME_SKEW_SECS) => {
                Duration::from_secs(0)
            }
            _ => {
                dbg.status_reason = Some("rollout mtime is in the future".into());
                return SessionStatus::Unknown;
            }
        },
    };

    // Very recent writes are a strong (but not perfect) signal of "working".
    if age <= Duration::from_secs(STATUS_WORKING_MAX_AGE_SECS) {
        dbg.status_reason = Some(format!("recent rollout write: {}s", age.as_secs()));
        return SessionStatus::Working;
    }

    // Rollouts do not persist all lifecycle events (e.g. TurnStarted/TurnComplete, RequestUserInput),
    // so even with tail hints we keep an mtime-based fallback that biases toward Unknown.
    if age <= Duration::from_secs(STATUS_UNCERTAIN_MAX_AGE_SECS) {
        dbg.status_reason = Some(format!(
            "uncertain (no rollout writes for {}s)",
            age.as_secs()
        ));
        return SessionStatus::Unknown;
    }

    dbg.status_reason = Some(format!("idle (no rollout writes for {}s)", age.as_secs()));
    SessionStatus::Waiting
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank_dbg() -> SessionDebug {
        SessionDebug {
            status_reason: None,
            process_command_sample: None,
            proc_cwd_source: None,
            meta_parse_error: None,
            meta_id_mismatch: None,
            repo_probe_error: None,
            title_source: None,
        }
    }

    #[test]
    fn classify_status_unknown_when_no_activity_time() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let mut dbg = blank_dbg();
        let status = classify_status(now, None, None, &mut dbg);
        assert!(matches!(status, SessionStatus::Unknown));
        assert_eq!(dbg.status_reason.as_deref(), Some("no rollout mtime"));
    }

    #[test]
    fn classify_status_tolerates_small_future_skew() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let last = now + Duration::from_secs(1);
        let mut dbg = blank_dbg();
        let status = classify_status(now, Some(last), None, &mut dbg);
        assert!(matches!(status, SessionStatus::Working));
    }

    #[test]
    fn classify_status_marks_large_future_skew_as_unknown() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let last = now + Duration::from_secs(STATUS_MAX_FUTURE_MTIME_SKEW_SECS + 5);
        let mut dbg = blank_dbg();
        let status = classify_status(now, Some(last), None, &mut dbg);
        assert!(matches!(status, SessionStatus::Unknown));
        assert_eq!(
            dbg.status_reason.as_deref(),
            Some("rollout mtime is in the future")
        );
    }

    #[test]
    fn classify_status_working_when_recent() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let last = now - Duration::from_secs(10);
        let mut dbg = blank_dbg();
        let status = classify_status(now, Some(last), None, &mut dbg);
        assert!(matches!(status, SessionStatus::Working));
    }

    #[test]
    fn classify_status_unknown_when_uncertain_window() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let last = now - Duration::from_secs(30);
        let mut dbg = blank_dbg();
        let status = classify_status(now, Some(last), None, &mut dbg);
        assert!(matches!(status, SessionStatus::Unknown));
    }

    #[test]
    fn classify_status_waiting_when_old() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let last = now - Duration::from_secs(STATUS_UNCERTAIN_MAX_AGE_SECS + 1);
        let mut dbg = blank_dbg();
        let status = classify_status(now, Some(last), None, &mut dbg);
        assert!(matches!(status, SessionStatus::Waiting));
    }

    #[test]
    fn classify_status_waiting_when_pending_user_input_call() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let last = now - Duration::from_secs(5);
        let mut dbg = blank_dbg();
        let pending = PendingFunctionCall {
            call_id: "call_ui".into(),
            name: "request_user_input".into(),
        };
        let status = classify_status(now, Some(last), Some(&pending), &mut dbg);
        assert!(matches!(status, SessionStatus::Waiting));
        assert_eq!(
            dbg.status_reason.as_deref(),
            Some("waiting for user input (call_id=call_ui)")
        );
    }

    #[test]
    fn classify_status_working_when_pending_tool_call() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let last = now - Duration::from_secs(STATUS_UNCERTAIN_MAX_AGE_SECS + 10);
        let mut dbg = blank_dbg();
        let pending = PendingFunctionCall {
            call_id: "call_exec".into(),
            name: "exec_command".into(),
        };
        let status = classify_status(now, Some(last), Some(&pending), &mut dbg);
        assert!(matches!(status, SessionStatus::Working));
        assert_eq!(
            dbg.status_reason.as_deref(),
            Some("pending tool call: exec_command (call_id=call_exec)")
        );
    }
}
