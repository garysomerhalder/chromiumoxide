//! The registry of bound browsers (Playwright's server registry behind
//! `browser.bind(title, { workspaceDir })`): one JSON descriptor per bound
//! browser, so tools can find the browsers of a workspace.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::domain::discovery::Os;

/// A browser some process has bound with [`bind`](crate::Browser::bind_with).
#[derive(Debug, Clone, PartialEq)]
pub struct BoundBrowser {
    /// The `title` given to `bind`.
    pub title: String,
    /// The WebSocket endpoint clients [`connect`](crate::Chromium::connect) to.
    pub endpoint: String,
    /// The `workspaceDir` given to `bind`.
    pub workspace_dir: Option<PathBuf>,
    /// The `metadata` given to `bind` (`null` if none).
    pub metadata: Value,
}

impl BoundBrowser {
    /// The descriptor file's contents.
    pub fn to_json(&self) -> String {
        let mut d = json!({
            "browserName": "chromium",
            "title": self.title,
            "endpoint": self.endpoint,
            "metadata": self.metadata,
        });
        if let Some(dir) = &self.workspace_dir {
            d["workspaceDir"] = json!(dir.to_string_lossy());
        }
        serde_json::to_string_pretty(&d).unwrap_or_default()
    }

    /// Reads a descriptor; `None` if it is not one.
    pub fn from_json(text: &str) -> Option<Self> {
        let d: Value = serde_json::from_str(text).ok()?;
        Some(Self {
            title: d.get("title")?.as_str()?.to_owned(),
            endpoint: d.get("endpoint")?.as_str()?.to_owned(),
            workspace_dir: d
                .get("workspaceDir")
                .and_then(Value::as_str)
                .map(PathBuf::from),
            metadata: d.get("metadata").cloned().unwrap_or(Value::Null),
        })
    }

    /// The descriptor's file name: a hash of the endpoint.
    pub fn file_name(&self) -> String {
        use std::hash::Hasher;
        let mut h = fnv::FnvHasher::default();
        h.write(self.endpoint.as_bytes());
        format!("{:016x}.json", h.finish())
    }
}

/// Where descriptors live: `CHROMIUMOXIDE_SERVER_REGISTRY`, else the
/// per-user cache directory plus `chromiumoxide/browsers`.
pub fn registry_dir(
    os: Os,
    var: impl Fn(&str) -> Option<String>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(dir) = var("CHROMIUMOXIDE_SERVER_REGISTRY").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let cache = match os {
        Os::Linux => var("XDG_CACHE_HOME")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".cache"))),
        Os::MacOs => home.map(|h| h.join("Library").join("Caches")),
        Os::Windows => var("LOCALAPPDATA").map(PathBuf::from),
    }?;
    Some(cache.join("chromiumoxide").join("browsers"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_round_trip() {
        let b = BoundBrowser {
            title: "t".into(),
            endpoint: "ws://127.0.0.1:1/x".into(),
            workspace_dir: Some("/w".into()),
            metadata: json!({"a": 1}),
        };
        assert_eq!(BoundBrowser::from_json(&b.to_json()), Some(b.clone()));
        assert!(b.file_name().ends_with(".json"));
        assert_eq!(BoundBrowser::from_json("{}"), None);
    }

    #[test]
    fn registry_location() {
        let none = |_: &str| None;
        let home = Path::new("/home/u");
        assert_eq!(
            registry_dir(Os::Linux, none, Some(home)),
            Some("/home/u/.cache/chromiumoxide/browsers".into())
        );
        let xdg = |k: &str| (k == "XDG_CACHE_HOME").then(|| "/c".to_owned());
        assert_eq!(
            registry_dir(Os::Linux, xdg, Some(home)),
            Some("/c/chromiumoxide/browsers".into())
        );
        let over = |k: &str| (k == "CHROMIUMOXIDE_SERVER_REGISTRY").then(|| "/r".to_owned());
        assert_eq!(registry_dir(Os::MacOs, over, None), Some("/r".into()));
        assert_eq!(
            registry_dir(Os::MacOs, none, Some(home)),
            Some("/home/u/Library/Caches/chromiumoxide/browsers".into())
        );
    }
}
