use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::model::SessionMeta;

#[derive(Debug, Deserialize)]
struct RolloutLine<T> {
    #[serde(rename = "type")]
    ty: String,
    payload: T,
}

#[derive(Debug, Deserialize)]
struct SessionMetaPayload {
    id: Option<String>,
    forked_from_id: Option<String>,
    cwd: Option<String>,
    source: Option<serde_json::Value>,
    git: Option<GitInfo>,
}

#[derive(Debug, Deserialize)]
struct GitInfo {
    commit_hash: Option<String>,
    branch: Option<String>,
}

pub fn read_session_meta(path: &Path) -> anyhow::Result<SessionMeta> {
    let f = File::open(path).with_context(|| format!("open rollout: {}", path.display()))?;
    let mut r = BufReader::new(f);
    let mut first = String::new();
    r.read_line(&mut first)
        .with_context(|| format!("read first line: {}", path.display()))?;

    let line: RolloutLine<SessionMetaPayload> =
        serde_json::from_str(&first).with_context(|| "parse first JSONL line")?;

    if line.ty != "session_meta" {
        anyhow::bail!(
            "expected first line type=session_meta, got type={}",
            line.ty
        );
    }

    let (session_source, subagent_parent_thread_id, subagent_depth) =
        parse_session_source(line.payload.source.as_ref());

    Ok(SessionMeta {
        id: line.payload.id,
        forked_from_id: line.payload.forked_from_id,
        cwd: line.payload.cwd,
        git_branch: line.payload.git.as_ref().and_then(|g| g.branch.clone()),
        git_commit: line
            .payload
            .git
            .as_ref()
            .and_then(|g| g.commit_hash.clone()),
        session_source,
        subagent_parent_thread_id,
        subagent_depth,
    })
}

fn parse_session_source(
    source: Option<&serde_json::Value>,
) -> (Option<String>, Option<String>, Option<i32>) {
    let Some(source) = source else {
        return (None, None, None);
    };

    match source {
        serde_json::Value::String(s) => (Some(s.clone()), None, None),
        serde_json::Value::Object(m) => {
            // For subagents, Codex serializes session source like:
            // {"subagent":{"thread_spawn":{"parent_thread_id":"...","depth":1}}}
            let Some(subagent) = m.get("subagent") else {
                return (Some("unknown".into()), None, None);
            };
            let Some(subagent_obj) = subagent.as_object() else {
                return (Some("subagent".into()), None, None);
            };
            let Some(thread_spawn) = subagent_obj.get("thread_spawn") else {
                return (Some("subagent".into()), None, None);
            };
            let Some(ts_obj) = thread_spawn.as_object() else {
                return (Some("subagent_thread_spawn".into()), None, None);
            };

            let parent = ts_obj
                .get("parent_thread_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let depth = ts_obj.get("depth").and_then(|v| v.as_i64()).map(|d| {
                // depth is small and expected to fit in i32.
                i32::try_from(d).unwrap_or(i32::MAX)
            });
            (Some("subagent_thread_spawn".into()), parent, depth)
        }
        _ => (Some("unknown".into()), None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn read_session_meta_parses_expected_fields() {
        let mut f = NamedTempFile::new().expect("tempfile");
        std::io::Write::write_all(
            &mut f,
            br#"{"type":"session_meta","payload":{"id":"019c2590-5605-7cd1-81b8-8a488af219a3","cwd":"/tmp/example","git":{"commit_hash":"abc123","branch":"main"}}}
"#,
        )
        .expect("write");

        let meta = read_session_meta(f.path()).expect("read_session_meta");
        assert_eq!(
            meta.id.as_deref(),
            Some("019c2590-5605-7cd1-81b8-8a488af219a3")
        );
        assert_eq!(meta.cwd.as_deref(), Some("/tmp/example"));
        assert_eq!(meta.git_branch.as_deref(), Some("main"));
        assert_eq!(meta.git_commit.as_deref(), Some("abc123"));
        assert_eq!(meta.session_source.as_deref(), None);
    }

    #[test]
    fn read_session_meta_parses_subagent_thread_spawn_source() {
        let mut f = NamedTempFile::new().expect("tempfile");
        std::io::Write::write_all(
            &mut f,
            br#"{"type":"session_meta","payload":{"id":"019c266f-631c-77c0-854f-2289c2d2fd8d","source":{"subagent":{"thread_spawn":{"parent_thread_id":"019c2590-5605-7cd1-81b8-8a488af219a3","depth":1}}}}}
"#,
        )
        .expect("write");

        let meta = read_session_meta(f.path()).expect("read_session_meta");
        assert_eq!(
            meta.session_source.as_deref(),
            Some("subagent_thread_spawn")
        );
        assert_eq!(
            meta.subagent_parent_thread_id.as_deref(),
            Some("019c2590-5605-7cd1-81b8-8a488af219a3")
        );
        assert_eq!(meta.subagent_depth, Some(1));
    }

    #[test]
    fn read_session_meta_requires_first_line_type() {
        let mut f = NamedTempFile::new().expect("tempfile");
        std::io::Write::write_all(
            &mut f,
            br#"{"type":"not_session_meta","payload":{}}
"#,
        )
        .expect("write");

        let err = read_session_meta(f.path()).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("expected first line type=session_meta"));
    }
}
