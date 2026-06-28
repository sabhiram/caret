//! Per-repo settings + secrets.
//!
//! Three buckets, deliberately separated:
//! - **Project settings** (`.caret/config.toml`) — committed; travels with the repo.
//! - **Secrets** (`.caret/secrets.toml`) — gitignored; values never leave the server.
//! - Personal/device overrides live in the browser (localStorage), not here.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub auto_save: bool,
    /// "dark" | "light" — the repo default; a viewer may override on their device.
    pub theme: String,
    /// Display title for the project; empty falls back to the directory name.
    pub title: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_save: false,
            theme: "dark".into(),
            title: String::new(),
        }
    }
}

fn config_path(dir: &Path) -> PathBuf {
    dir.join(".caret").join("config.toml")
}
fn secrets_path(dir: &Path) -> PathBuf {
    dir.join(".caret").join("secrets.toml")
}

pub fn load(dir: &Path) -> Settings {
    std::fs::read_to_string(config_path(dir))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, s: &Settings) -> Result<()> {
    let p = config_path(dir);
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, toml::to_string_pretty(s)?)?;
    Ok(())
}

#[derive(Serialize, Deserialize, Default)]
struct Secrets {
    #[serde(default)]
    keys: BTreeMap<String, String>,
}

fn load_secrets(dir: &Path) -> Secrets {
    std::fs::read_to_string(secrets_path(dir))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// Names of secrets currently set. Values are intentionally NOT returned.
pub fn secret_names(dir: &Path) -> Vec<String> {
    load_secrets(dir).keys.into_keys().collect()
}

/// Set (or, with an empty value, clear) a secret. Ensures the secrets file is
/// gitignored before writing, so a key can never land in a commit.
pub fn set_secret(dir: &Path, name: &str, value: &str) -> Result<()> {
    ensure_gitignored(dir)?;
    let mut s = load_secrets(dir);
    if value.is_empty() {
        s.keys.remove(name);
    } else {
        s.keys.insert(name.to_string(), value.to_string());
    }
    let p = secrets_path(dir);
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, toml::to_string_pretty(&s)?)?;
    Ok(())
}

/// Append `.caret/secrets.toml` to `.gitignore` if it isn't already ignored.
fn ensure_gitignored(dir: &Path) -> Result<()> {
    let needle = ".caret/secrets.toml";
    let gi = dir.join(".gitignore");
    let mut content = std::fs::read_to_string(&gi).unwrap_or_default();
    if content.lines().any(|l| l.trim() == needle) {
        return Ok(());
    }
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(needle);
    content.push('\n');
    std::fs::write(&gi, content)?;
    Ok(())
}

/// The blob injected into the page: (theme, json) where json carries the
/// client-visible settings — including which secrets are set, never their values.
pub fn client_blob(dir: &Path) -> (String, String) {
    let s = load(dir);
    let theme = if s.theme == "light" { "light" } else { "dark" }.to_string();
    let basename = dir_basename(dir);
    let project = if s.title.trim().is_empty() {
        basename.clone()
    } else {
        s.title.trim().to_string()
    };
    let json = serde_json::json!({
        "autoSave": s.auto_save,
        "theme": theme,
        "project": project,
        "title": s.title,
        "basename": basename,
        "secrets": secret_names(dir),
    })
    .to_string();
    (theme, json)
}

/// The served directory's name (e.g. "my-docs") — the project-title fallback.
fn dir_basename(dir: &Path) -> String {
    std::fs::canonicalize(dir)
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "docs".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_secret_isolation() {
        let tmp = std::env::temp_dir().join(format!("caret-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // defaults
        let d = load(&tmp);
        assert!(!d.auto_save && d.theme == "dark");

        // settings persist
        save(
            &tmp,
            &Settings {
                auto_save: true,
                theme: "light".into(),
                title: "My Project".into(),
            },
        )
        .unwrap();
        let l = load(&tmp);
        assert!(l.auto_save && l.theme == "light" && l.title == "My Project");
        // title drives the resolved project name in the client blob
        assert!(client_blob(&tmp).1.contains("My Project"));

        // secret is stored but gitignored and never surfaced by value
        set_secret(&tmp, "anthropic", "sk-test-123").unwrap();
        assert_eq!(secret_names(&tmp), vec!["anthropic".to_string()]);
        let gi = std::fs::read_to_string(tmp.join(".gitignore")).unwrap();
        assert!(
            gi.contains(".caret/secrets.toml"),
            "secrets must be gitignored"
        );
        let (_t, json) = client_blob(&tmp);
        assert!(
            !json.contains("sk-test-123"),
            "secret value must never reach the client"
        );

        // clearing removes it
        set_secret(&tmp, "anthropic", "").unwrap();
        assert!(secret_names(&tmp).is_empty());

        std::fs::remove_dir_all(&tmp).ok();
    }
}
