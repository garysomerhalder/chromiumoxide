//! Builder setters derived from array fields must not collide with real fields
//! (Animation.seekAnimations has both `currentTime` and `currentTimes`).

use chromiumoxide_pdl::build::Generator;

const PDL: &str = "version
  major 1
  minor 3

domain Demo
  command seek
    parameters
      optional number currentTime
      optional array of number currentTimes
";

#[test]
fn singular_setter_is_skipped_when_a_field_has_that_name() {
    let dir = std::env::temp_dir().join(format!("pdl-builder-names-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pdl = dir.join("demo.pdl");
    std::fs::write(&pdl, PDL).unwrap();
    Generator::default()
        .out_dir(&dir)
        .compile_pdls(&[pdl])
        .unwrap();
    let code = std::fs::read_to_string(dir.join("cdp.rs")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(
        compact.matches("pubfncurrent_time(").count(),
        1,
        "duplicate `current_time` setter"
    );
    assert_eq!(
        compact.matches("pubfncurrent_times<").count(),
        1,
        "iterator setter missing"
    );
}
