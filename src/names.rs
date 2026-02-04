use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct SessionNameKey {
    pub host: String,
    pub thread_id: String,
}

#[derive(Clone, Debug)]
pub struct NamesStore {
    path: PathBuf,
    last_mtime: Option<SystemTime>,
    names: HashMap<SessionNameKey, String>,
}

impl NamesStore {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self::new_at(default_names_path()?))
    }

    fn new_at(path: PathBuf) -> Self {
        Self {
            path,
            last_mtime: None,
            names: HashMap::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn refresh_if_changed(&mut self) -> anyhow::Result<()> {
        let meta = match fs::metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.last_mtime = None;
                self.names.clear();
                return Ok(());
            }
            Err(e) => {
                return Err(e).with_context(|| format!("stat {}", self.path.display()));
            }
        };

        let mtime = meta.modified().ok();
        if mtime.is_some() && self.last_mtime.is_some() && mtime == self.last_mtime {
            return Ok(());
        }

        let parsed: anyhow::Result<HashMap<SessionNameKey, String>> = (|| {
            let f = fs::File::open(&self.path)
                .with_context(|| format!("open {}", self.path.display()))?;
            let mut r = BufReader::new(f);

            let mut names: HashMap<SessionNameKey, String> = HashMap::new();

            let mut line = String::new();
            let mut line_no: usize = 0;
            while r.read_line(&mut line).context("read line")? > 0 {
                line_no += 1;
                let raw = line.trim().to_string();
                line.clear();

                if raw.is_empty() {
                    continue;
                }

                let rec: NamesLine = serde_json::from_str(&raw)
                    .with_context(|| format!("parse session_names.jsonl line {line_no}"))?;

                let key = SessionNameKey {
                    host: rec.host,
                    thread_id: rec.thread_id,
                };

                match normalize_name_opt(rec.name) {
                    Some(name) => {
                        names.insert(key, name);
                    }
                    None => {
                        names.remove(&key);
                    }
                }
            }

            Ok(names)
        })();

        match parsed {
            Ok(names) => {
                self.names = names;
                self.last_mtime = mtime;
                Ok(())
            }
            Err(e) => {
                self.names.clear();
                self.last_mtime = mtime;
                Err(e)
            }
        }
    }

    pub fn get_cached(&self, key: &SessionNameKey) -> Option<&str> {
        self.names.get(key).map(|s| s.as_str())
    }

    pub fn set(&mut self, key: SessionNameKey, name: String) -> anyhow::Result<Option<String>> {
        let Some(normalized) = normalize_name_opt(Some(name)) else {
            self.clear(key)?;
            return Ok(None);
        };

        self.append_record(&key, Some(&normalized))?;
        self.names.insert(key, normalized.clone());
        Ok(Some(normalized))
    }

    pub fn clear(&mut self, key: SessionNameKey) -> anyhow::Result<()> {
        self.append_record(&key, None)?;
        self.names.remove(&key);
        Ok(())
    }

    fn append_record(&mut self, key: &SessionNameKey, name: Option<&str>) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create dir {}", parent.display()))?;
        }

        let rec = NamesLine {
            host: key.host.clone(),
            thread_id: key.thread_id.clone(),
            name: name.map(|s| s.to_string()),
        };
        let line = serde_json::to_string(&rec).with_context(|| "serialize session name record")?;

        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("open for append {}", self.path.display()))?;
        writeln!(f, "{line}").with_context(|| "append session name record")?;
        f.flush().ok();

        // Best-effort mtime update to keep the cache fresh without rereading.
        self.last_mtime = fs::metadata(&self.path)
            .ok()
            .and_then(|m| m.modified().ok());
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NamesLine {
    host: String,
    thread_id: String,
    name: Option<String>,
}

fn normalize_name_opt(name: Option<String>) -> Option<String> {
    let name = name?;
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn default_names_path() -> anyhow::Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let xdg = xdg.trim();
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg)
                .join("codex-ps")
                .join("session_names.jsonl"));
        }
    }

    let home = dirs::home_dir().context("resolve home dir (needed for ~/.config)")?;
    Ok(home.join(".config/codex-ps/session_names.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn refresh_prefers_latest_entry_and_supports_clear() {
        let dir = TempDir::new().expect("tempdir");
        let p = dir.path().join("session_names.jsonl");
        fs::write(
            &p,
            r#"{"host":"local","thread_id":"t1","name":"first"}
{"host":"local","thread_id":"t1","name":"second"}
{"host":"local","thread_id":"t2","name":"x"}
{"host":"local","thread_id":"t1","name":null}
{"host":"local","thread_id":"t1","name":"final"}
"#,
        )
        .expect("write");

        let mut store = NamesStore::new_at(p);
        store.refresh_if_changed().expect("refresh");

        let k1 = SessionNameKey {
            host: "local".into(),
            thread_id: "t1".into(),
        };
        let k2 = SessionNameKey {
            host: "local".into(),
            thread_id: "t2".into(),
        };
        assert_eq!(store.get_cached(&k1), Some("final"));
        assert_eq!(store.get_cached(&k2), Some("x"));
    }

    #[test]
    fn clears_cache_when_file_disappears() {
        let dir = TempDir::new().expect("tempdir");
        let p = dir.path().join("session_names.jsonl");
        fs::write(&p, r#"{"host":"local","thread_id":"t1","name":"x"}"#).expect("write");

        let mut store = NamesStore::new_at(p.clone());
        store.refresh_if_changed().expect("refresh");
        let k1 = SessionNameKey {
            host: "local".into(),
            thread_id: "t1".into(),
        };
        assert_eq!(store.get_cached(&k1), Some("x"));

        fs::remove_file(&p).expect("remove");
        store.refresh_if_changed().expect("refresh");
        assert_eq!(store.get_cached(&k1), None);
    }

    #[test]
    fn set_trims_and_writes_record() {
        let dir = TempDir::new().expect("tempdir");
        let p = dir.path().join("session_names.jsonl");

        let mut store = NamesStore::new_at(p.clone());
        let key = SessionNameKey {
            host: "local".into(),
            thread_id: "t1".into(),
        };
        assert_eq!(
            store
                .set(key.clone(), "  hello world  ".into())
                .expect("set"),
            Some("hello world".into())
        );
        assert_eq!(store.get_cached(&key), Some("hello world"));

        let bytes = fs::read_to_string(&p).expect("read");
        assert!(bytes.contains(r#""name":"hello world""#));
    }

    #[test]
    fn set_empty_string_behaves_like_clear() {
        let dir = TempDir::new().expect("tempdir");
        let p = dir.path().join("session_names.jsonl");

        let mut store = NamesStore::new_at(p.clone());
        let key = SessionNameKey {
            host: "local".into(),
            thread_id: "t1".into(),
        };
        store.set(key.clone(), "x".into()).expect("set x");
        assert_eq!(store.get_cached(&key), Some("x"));

        assert_eq!(store.set(key.clone(), "   ".into()).expect("set"), None);
        assert_eq!(store.get_cached(&key), None);
    }
}
