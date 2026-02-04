use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Context;
use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct TitleResolver {
    path: PathBuf,
    last_mtime: Option<SystemTime>,
    titles: HashMap<String, String>,
}

impl TitleResolver {
    pub fn new(codex_home: &Path) -> Self {
        Self {
            path: codex_home.join(".codex-global-state.json"),
            last_mtime: None,
            titles: HashMap::new(),
        }
    }

    pub fn get_title(&mut self, thread_id: &str) -> anyhow::Result<Option<(String, &'static str)>> {
        self.refresh_if_changed()?;
        if let Some(t) = self.titles.get(thread_id) {
            Ok(Some((t.clone(), "codex-global-state.json")))
        } else {
            Ok(None)
        }
    }

    fn refresh_if_changed(&mut self) -> anyhow::Result<()> {
        let meta = match fs::metadata(&self.path) {
            Ok(m) => m,
            Err(_) => {
                // If the titles file disappears, treat it as unavailable (don't keep stale cache).
                self.last_mtime = None;
                self.titles.clear();
                return Ok(());
            }
        };
        let mtime = meta.modified().ok();
        if mtime.is_some() && self.last_mtime.is_some() && mtime == self.last_mtime {
            return Ok(());
        }

        let bytes =
            fs::read(&self.path).with_context(|| format!("read {}", self.path.display()))?;
        let parsed: GlobalState =
            serde_json::from_slice(&bytes).with_context(|| "parse codex global state JSON")?;

        self.titles = parsed
            .thread_titles
            .and_then(|tt| tt.titles)
            .unwrap_or_default();
        self.last_mtime = mtime;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct GlobalState {
    #[serde(rename = "thread-titles")]
    thread_titles: Option<ThreadTitles>,
}

#[derive(Debug, Deserialize)]
struct ThreadTitles {
    titles: Option<HashMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn resolves_title_from_global_state() {
        let dir = TempDir::new().expect("tempdir");
        let p = dir.path().join(".codex-global-state.json");
        fs::write(
            &p,
            r#"{"thread-titles":{"titles":{"019c2590-5605-7cd1-81b8-8a488af219a3":"Hello"}}}"#,
        )
        .expect("write global state");

        let mut r = TitleResolver::new(dir.path());
        let (title, src) = r
            .get_title("019c2590-5605-7cd1-81b8-8a488af219a3")
            .expect("get_title")
            .expect("title present");
        assert_eq!(title, "Hello");
        assert_eq!(src, "codex-global-state.json");
    }

    #[test]
    fn returns_none_for_unknown_thread() {
        let dir = TempDir::new().expect("tempdir");
        let p = dir.path().join(".codex-global-state.json");
        fs::write(&p, r#"{"thread-titles":{"titles":{}}}"#).expect("write");

        let mut r = TitleResolver::new(dir.path());
        assert!(r.get_title("missing").expect("get_title").is_none());
    }

    #[test]
    fn clears_cache_when_global_state_disappears() {
        let dir = TempDir::new().expect("tempdir");
        let p = dir.path().join(".codex-global-state.json");
        fs::write(
            &p,
            r#"{"thread-titles":{"titles":{"019c2590-5605-7cd1-81b8-8a488af219a3":"Hello"}}}"#,
        )
        .expect("write global state");

        let mut r = TitleResolver::new(dir.path());
        assert!(
            r.get_title("019c2590-5605-7cd1-81b8-8a488af219a3")
                .expect("get_title")
                .is_some()
        );

        fs::remove_file(&p).expect("remove global state");
        assert!(
            r.get_title("019c2590-5605-7cd1-81b8-8a488af219a3")
                .expect("get_title")
                .is_none()
        );
    }
}
