use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::util::run_cmd_with_timeout;

#[derive(Clone, Debug)]
pub struct GitCache {
    ttl: Duration,
    entries: HashMap<PathBuf, (Instant, Option<PathBuf>)>,
}

impl GitCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: HashMap::new(),
        }
    }

    pub fn repo_root(
        &mut self,
        cwd: &Path,
        timeout: Duration,
    ) -> anyhow::Result<(Option<PathBuf>, Option<String>)> {
        let now = Instant::now();
        if let Some((ts, cached)) = self.entries.get(cwd) {
            if now.duration_since(*ts) <= self.ttl {
                return Ok((cached.clone(), None));
            }
        }

        let mut cmd = Command::new("git");
        cmd.args([
            "-C",
            cwd.to_string_lossy().as_ref(),
            "rev-parse",
            "--show-toplevel",
        ]);
        let out = match run_cmd_with_timeout(cmd, timeout) {
            Ok(o) if o.status.success() => o,
            Ok(_) => {
                self.entries.insert(cwd.to_path_buf(), (now, None));
                return Ok((None, Some("git rev-parse failed".into())));
            }
            Err(e) => {
                self.entries.insert(cwd.to_path_buf(), (now, None));
                return Ok((None, Some(format!("{e}"))));
            }
        };

        let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if root.is_empty() {
            self.entries.insert(cwd.to_path_buf(), (now, None));
            return Ok((None, Some("git rev-parse returned empty".into())));
        }

        let pb = PathBuf::from(root);
        self.entries
            .insert(cwd.to_path_buf(), (now, Some(pb.clone())));
        Ok((Some(pb), None))
    }
}
