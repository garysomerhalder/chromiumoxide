//! Aria snapshot templates (`toMatchAriaSnapshot`): a port of Playwright's
//! `parseAriaSnapshot`, with the YAML subset aria snapshots use, producing
//! the template object the injected script's `matchesExpectAriaTemplate`
//! takes.

use serde_json::Value;

/// Parses a template; the error is Playwright's first error message.
pub fn parse_aria_template(text: &str) -> Result<Value, String> {
    unimplemented!()
}

/// Playwright's `unshift`: drops blank lines and the common indentation.
pub fn unshift(snapshot: &str) -> String {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inputs and the results of Playwright 1.63's `parseAriaSnapshot`.
    const CASES: &str = include_str!("data/aria_template_cases.json");

    #[test]
    fn parses_like_playwright() {
        let cases: Vec<Value> = serde_json::from_str(CASES).unwrap();
        for case in cases {
            let input = case["input"].as_str().unwrap();
            let got = parse_aria_template(input);
            match case["errors"].as_array().and_then(|e| e.first()) {
                Some(err) => assert_eq!(got, Err(err.as_str().unwrap().to_owned()), "{input}"),
                None => assert_eq!(got, Ok(case["fragment"].clone()), "{input}"),
            }
        }
    }

    #[test]
    fn unshift_removes_common_indent_and_blank_lines() {
        assert_eq!(
            unshift("\n      - list:\n        - listitem: a\n\n      - button\n    "),
            "- list:\n  - listitem: a\n- button"
        );
    }
}
