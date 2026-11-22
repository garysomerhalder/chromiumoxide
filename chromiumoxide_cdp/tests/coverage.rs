//! The generated protocol covers the vendored PDL and the latest revision.

use std::fs;
use std::path::Path;

use chromiumoxide_cdp::CURRENT_REVISION;

fn snake(name: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = name.chars().collect();
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

/// `domain Foo` declarations in the vendored PDL (top-level files and `domains/`).
fn pdl_domains() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("pdl");
    let mut files = vec![
        dir.join("js_protocol.pdl"),
        dir.join("browser_protocol.pdl"),
    ];
    if let Ok(rd) = fs::read_dir(dir.join("domains")) {
        files.extend(rd.filter_map(|e| e.ok()).map(|e| e.path()));
    }
    let mut domains = Vec::new();
    for f in files {
        for line in fs::read_to_string(&f).unwrap().lines() {
            let line = line
                .trim_start_matches("experimental ")
                .trim_start_matches("deprecated ");
            if let Some(name) = line.strip_prefix("domain ") {
                domains.push(name.trim().to_owned());
            }
        }
    }
    domains.sort();
    domains.dedup();
    domains
}

#[test]
fn every_vendored_domain_is_generated() {
    let generated =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cdp.rs")).unwrap();
    let domains = pdl_domains();
    assert!(domains.len() > 50, "found only {} domains", domains.len());
    let missing: Vec<_> = domains
        .iter()
        .filter(|d| !generated.contains(&format!("pub mod {} {{", snake(d))))
        .collect();
    assert!(
        missing.is_empty(),
        "domains without a generated module: {missing:?}"
    );
}

#[test]
fn protocol_is_the_latest_revision() {
    // ChromeDevTools/devtools-protocol tag v0.0.1751165 (2026-11).
    assert_eq!(CURRENT_REVISION.to_string(), "v0.0.1751165");
    for d in [
        "Ads",
        "CrashReportContext",
        "DigitalCredentials",
        "FindInPage",
        "SmartCardEmulation",
        "WebMCP",
    ] {
        assert!(
            pdl_domains().contains(&d.to_owned()),
            "{d} missing from vendored PDL"
        );
    }
}
