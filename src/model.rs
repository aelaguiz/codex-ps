use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub generated_at_unix_s: i64,
    pub host: String,
    pub sessions: Vec<SessionRow>,
    pub host_errors: Option<Vec<HostError>>,
    pub warnings: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRow {
    #[serde(default)]
    pub host: String,
    pub thread_id: String,
    pub pids: Vec<i32>,
    pub tty: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub repo_root: Option<String>,
    pub git_branch: Option<String>,
    pub git_commit: Option<String>,
    /// Best-effort source/role hint from `session_meta.source` (e.g. "cli", "vscode", "subagent").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_source: Option<String>,
    /// Best-effort lineage hint from `session_meta.forked_from_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from_id: Option<String>,
    /// Present when this thread was spawned as a subagent (thread spawn) and has a parent thread id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_parent_thread_id: Option<String>,
    /// Subagent spawn depth when present (0=root).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_depth: Option<i32>,
    pub status: SessionStatus,
    pub last_activity_unix_s: Option<i64>,
    pub rollout_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<SessionDebug>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostError {
    pub host: String,
    pub error: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Working,
    Waiting,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionDebug {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_command_sample: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proc_cwd_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta_parse_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta_id_mismatch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_probe_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_source: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SessionMeta {
    pub id: Option<String>,
    pub forked_from_id: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub git_commit: Option<String>,
    pub session_source: Option<String>,
    pub subagent_parent_thread_id: Option<String>,
    pub subagent_depth: Option<i32>,
}

#[derive(Clone, Debug)]
pub struct SessionBuilder {
    pub thread_id: String,
    pub pids: Vec<i32>,
    pub tty: Option<String>,
    pub proc_cwd: Option<PathBuf>,
    pub rollout_path: Option<PathBuf>,
    pub proc_command_sample: Option<String>,
}
