//! Coverage check for the typed per-domain API (`cdp::domains`).
//!
//! Every non-deprecated command and event of every non-deprecated domain in
//! the vendored protocol must have a method on its domain's client, or be
//! listed in [`EXCLUDED`] with a reason (mirrored in `docs/PARITY.md`). Every
//! domain must also have an integration test against real Chromium in
//! `tests/integration/cdp_domains.rs`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use chromiumoxide_cdp::cdp::domains;
use chromiumoxide_pdl::pdl::parser::parse_pdl;
use chromiumoxide_pdl::pdl::resolver::resolve_pdl;

/// `Domain.method` entries that deliberately have no typed method, with why.
const EXCLUDED: &[(&str, &str)] = &[];

fn pdl_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("pdl")
}

/// `(domain, Domain.method)` of every non-deprecated command and event.
fn protocol_methods() -> (BTreeSet<String>, BTreeSet<String>) {
    let mut domains_seen = BTreeSet::new();
    let mut methods = BTreeSet::new();
    for file in ["js_protocol.pdl", "browser_protocol.pdl"] {
        let path = pdl_dir().join(file);
        let input = fs::read_to_string(&path).unwrap();
        let resolved = resolve_pdl(&path, &input).unwrap();
        let protocol = parse_pdl(&resolved).unwrap();
        for domain in protocol.domains.iter().filter(|d| !d.deprecated) {
            domains_seen.insert(domain.name.to_string());
            for cmd in domain.commands.iter().filter(|c| !c.deprecated) {
                methods.insert(format!("{}.{}", domain.name, cmd.name));
            }
            for ev in domain.events.iter().filter(|e| !e.deprecated) {
                methods.insert(format!("{}.{}", domain.name, ev.name));
            }
        }
    }
    (domains_seen, methods)
}

#[test]
fn every_command_and_event_has_a_typed_method() {
    let (domains_seen, methods) = protocol_methods();
    assert!(
        domains_seen.len() > 50,
        "only {} domains",
        domains_seen.len()
    );
    let covered: BTreeSet<&str> = domains::COVERED.iter().map(|(m, _)| *m).collect();
    let excluded: BTreeSet<&str> = EXCLUDED.iter().map(|(m, _)| *m).collect();
    let missing: Vec<&String> = methods
        .iter()
        .filter(|m| !covered.contains(m.as_str()) && !excluded.contains(m.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "{} commands/events have no typed method: {missing:?}",
        missing.len()
    );
    let stale: Vec<&&str> = covered.iter().filter(|m| !methods.contains(**m)).collect();
    assert!(
        stale.is_empty(),
        "typed methods not in the protocol: {stale:?}"
    );
}

#[test]
fn every_domain_has_an_integration_test() {
    let (domains_seen, _) = protocol_methods();
    let tests = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/integration/cdp_domains.rs"),
    )
    .unwrap();
    let missing: Vec<String> = domains_seen
        .iter()
        .filter(|d| {
            let accessor = domains::DOMAINS.iter().find(|(name, _)| name == d);
            !accessor.is_some_and(|(_, a)| tests.contains(&format!("async fn cdp_{a}()")))
        })
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "domains without an integration test in tests/integration/cdp_domains.rs: {missing:?}"
    );
}
