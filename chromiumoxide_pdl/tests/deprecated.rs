//! Deprecated protocol items generate valid Rust when they are included.

use chromiumoxide_pdl::build::Generator;

const PDL: &str = "version
  major 1
  minor 3

domain Demo
  deprecated event oldThing
    parameters
      string reason
  event newThing
    parameters
      string reason
";

#[test]
fn deprecated_event_variants_get_a_real_attribute() {
    let dir = std::env::temp_dir().join(format!("pdl-deprecated-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pdl = dir.join("demo.pdl");
    std::fs::write(&pdl, PDL).unwrap();
    Generator::default()
        .out_dir(&dir)
        .deprecated(true)
        .compile_pdls(&[pdl])
        .unwrap();
    let code = std::fs::read_to_string(dir.join("cdp.rs")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !code.contains(", [deprecated]") && !code.contains("{ [deprecated]"),
        "bare `[deprecated]` emitted"
    );
    assert!(
        code.contains("# [deprecated] DemoOldThing"),
        "variant not marked deprecated"
    );
}
