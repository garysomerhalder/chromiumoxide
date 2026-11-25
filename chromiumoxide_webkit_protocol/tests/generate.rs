//! The committed `src/webkit.rs` is what the generator makes of the
//! vendored `protocol/protocol.json` (Playwright's WebKit build).

use std::fs;
use std::path::Path;

use chromiumoxide_pdl::build::Generator;

#[test]
fn generated_code_is_fresh() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tmp = tempfile::tempdir().unwrap();
    Generator::default()
        .out_dir(tmp.path())
        .target_mod("webkit")
        .docs_url(None)
        .compile_protocol_json(dir.join("protocol/protocol.json"))
        .unwrap();
    let new = fs::read_to_string(tmp.path().join("webkit.rs")).unwrap();
    let src = dir.join("src/webkit.rs");
    let old = fs::read_to_string(&src).unwrap_or_default();
    if new != old {
        fs::write(src, new).unwrap();
        panic!("generated code in the repository is outdated, updating...");
    }
}

#[test]
fn every_domain_is_generated() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("protocol/protocol.json")).unwrap())
            .unwrap();
    let generated = fs::read_to_string(dir.join("src/webkit.rs")).unwrap_or_default();
    let domains: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["domain"].as_str().unwrap())
        .collect();
    assert_eq!(domains.len(), 33);
    for d in [
        "Playwright",
        "Page",
        "Runtime",
        "Network",
        "Emulation",
        "Target",
    ] {
        assert!(domains.contains(&d), "{d}");
    }
    let missing: Vec<_> = domains
        .iter()
        .filter(|d| {
            let snake = chromiumoxide_webkit_protocol_snake(d);
            !generated.contains(&format!("pub mod {snake} {{"))
        })
        .collect();
    assert!(missing.is_empty(), "{missing:?}");
}

fn chromiumoxide_webkit_protocol_snake(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev_lower = i > 0 && chars[i - 1].is_lowercase();
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let prev_upper = i > 0 && chars[i - 1].is_uppercase();
            if i > 0 && (prev_lower || (prev_upper && next_lower)) {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(*c);
        }
    }
    out
}
