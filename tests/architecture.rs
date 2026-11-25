//! Enforces the hexagonal dependency rule: dependencies point inward.
//!
//! domain  <-  ports  <-  application  <-  adapters
use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    assert!(dir.is_dir(), "missing layer directory {}", dir.display());
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

fn assert_layer(layer: &str, forbidden: &[&str]) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(layer);
    let mut violations = Vec::new();
    for file in rust_files(&root) {
        let text = fs::read_to_string(&file).unwrap();
        for (no, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            for f in forbidden {
                if code.contains(f) {
                    violations.push(format!("{}:{}: uses `{f}`", file.display(), no + 1));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "layer `{layer}` breaks the dependency rule:\n{}",
        violations.join("\n")
    );
}

#[test]
fn domain_is_pure() {
    assert_layer(
        "domain",
        &[
            "tokio",
            "chromiumoxide_cdp",
            "chromiumoxide_webkit_protocol",
            "async_tungstenite",
            "crate::ports",
            "crate::application",
            "crate::adapters",
            "std::fs",
            "std::process",
            "std::net",
            "std::env",
            "std::time::Instant",
            "SystemTime",
        ],
    );
}

#[test]
fn ports_depend_only_on_domain() {
    assert_layer(
        "ports",
        &[
            "tokio",
            "chromiumoxide_cdp",
            "chromiumoxide_webkit_protocol",
            "crate::application",
            "crate::adapters",
            "std::fs",
            "std::process",
        ],
    );
}

#[test]
fn application_does_not_know_adapters() {
    assert_layer(
        "application",
        &[
            "crate::adapters",
            "std::fs",
            "std::process",
            "std::env",
            "tokio::fs",
            "tokio::process",
            "tokio::net",
            "tokio::time",
            "async_tungstenite",
        ],
    );
}
