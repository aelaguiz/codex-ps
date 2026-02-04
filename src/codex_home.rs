use anyhow::Context;

#[derive(Clone, Debug)]
pub struct CodexHome {
    pub root: std::path::PathBuf,
}

impl CodexHome {
    pub fn resolve(override_path: Option<std::path::PathBuf>) -> anyhow::Result<Self> {
        if let Some(p) = override_path {
            return Ok(Self { root: p });
        }

        if let Ok(env) = std::env::var("CODEX_HOME") {
            if !env.trim().is_empty() {
                return Ok(Self {
                    root: std::path::PathBuf::from(env),
                });
            }
        }

        let home = dirs::home_dir().context("resolve home dir (needed for ~/.codex)")?;
        Ok(Self {
            root: home.join(".codex"),
        })
    }
}
