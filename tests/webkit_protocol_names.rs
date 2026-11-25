//! Every WebKit command and event name the engine uses exists in WebKit's
//! generated protocol (`chromiumoxide_webkit_protocol`), so a typo or a
//! method a new WebKit build drops fails here, not in a browser.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const DOMAINS: &[&str] = &[
    "Playwright",
    "Page",
    "Runtime",
    "DOM",
    "Network",
    "Emulation",
    "Target",
    "Input",
    "Worker",
    "Heap",
    "Browser",
    "Screencast",
    "Dialog",
    "Console",
];

/// Names that are not methods: a file name and `starts_with` prefixes.
const NOT_METHODS: &[&str] = &["Playwright.exe", "Playwright.download", "Network.webSocket"];

#[test]
fn webkit_engine_uses_only_protocol_methods() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let generated =
        fs::read_to_string(root.join("chromiumoxide_webkit_protocol/src/webkit.rs")).unwrap();
    let known: BTreeSet<&str> = generated
        .split("IDENTIFIER: &'static str = \"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .collect();
    assert!(
        known.contains("Playwright.navigate"),
        "{} names",
        known.len()
    );

    let mut unknown = Vec::new();
    for entry in fs::read_dir(root.join("src/application")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.starts_with("webkit") || !name.ends_with(".rs") {
            continue;
        }
        let src = fs::read_to_string(&path).unwrap();
        for quoted in src.split('"').skip(1).step_by(2) {
            let Some((domain, method)) = quoted.split_once('.') else {
                continue;
            };
            if !DOMAINS.contains(&domain)
                || method.is_empty()
                || !method.chars().all(|c| c.is_ascii_alphanumeric())
                || NOT_METHODS.contains(&quoted)
            {
                continue;
            }
            if !known.contains(quoted) {
                unknown.push(format!("{name}: {quoted}"));
            }
        }
    }
    assert!(unknown.is_empty(), "not in WebKit's protocol: {unknown:#?}");
}
