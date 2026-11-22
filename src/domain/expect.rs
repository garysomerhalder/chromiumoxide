//! Web-first assertions: the expected-value encoding Playwright's injected
//! `expect()` takes, and Playwright's failure messages.

use serde_json::{Value, json};

use crate::domain::selectors::TextMatch;

/// How an expected text is compared (Playwright's `ExpectedTextValue` flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextOptions {
    pub match_substring: bool,
    pub normalize_white_space: bool,
    pub ignore_case: Option<bool>,
}

impl TextOptions {
    /// `toHaveText`, `toHaveTitle`, accessible name/description.
    pub const HAVE: TextOptions = TextOptions {
        match_substring: false,
        normalize_white_space: true,
        ignore_case: None,
    };
    /// `toContainText`.
    pub const CONTAIN: TextOptions = TextOptions {
        match_substring: true,
        normalize_white_space: true,
        ignore_case: None,
    };
    /// Values compared as they are (`toHaveValue`, `toHaveURL`, attributes, ...).
    pub const EXACT: TextOptions = TextOptions {
        match_substring: false,
        normalize_white_space: false,
        ignore_case: None,
    };
}

/// One `ExpectedTextValue` for the injected `expect()`.
pub fn expected_text(m: &TextMatch, o: TextOptions) -> Value {
    let mut v = match m {
        TextMatch::Text(s) => json!({"string": s}),
        TextMatch::Regex { pattern, flags } => json!({"regexSource": pattern, "regexFlags": flags}),
    };
    v["matchSubstring"] = json!(o.match_substring);
    v["normalizeWhiteSpace"] = json!(o.normalize_white_space);
    if let Some(ic) = o.ignore_case {
        v["ignoreCase"] = json!(ic);
    }
    v
}

/// How Playwright prints an expected text: `"text"` or `/regex/flags`.
pub fn display_expected(m: &TextMatch) -> String {
    match m {
        TextMatch::Text(s) => quote(s),
        TextMatch::Regex { pattern, flags } => format!("/{pattern}/{flags}"),
    }
}

/// A JSON-style quoted string.
pub fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// What an assertion is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// With the locator's description, e.g. `locator('#a')`.
    Locator(String),
    Page,
    Response,
}

/// A failed assertion, formatted like Playwright's error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub subject: Subject,
    /// e.g. `toHaveText(expected)`.
    pub matcher: String,
    pub is_not: bool,
    pub expected: String,
    pub received: Option<String>,
    pub timeout_ms: u64,
    /// The locator matched nothing.
    pub not_found: bool,
}

impl Failure {
    pub fn message(&self) -> String {
        let who = match self.subject {
            Subject::Locator(_) => "locator",
            Subject::Page => "page",
            Subject::Response => "response",
        };
        let not = if self.is_not { "not." } else { "" };
        let mut out = format!("expect({who}).{not}{} failed\n\n", self.matcher);
        // Playwright aligns the labels when a "Received:" line is printed.
        let wide = !self.not_found && self.received.is_some();
        let pad = |label: &str| {
            if wide {
                format!("{label:<10}")
            } else {
                format!("{label} ")
            }
        };
        if let Subject::Locator(l) = &self.subject {
            out.push_str(&format!("{}{l}\n", pad("Locator:")));
        }
        out.push_str(&format!("{}{}\n", pad("Expected:"), self.expected));
        if let Some(r) = self.received.as_ref().filter(|_| !self.not_found) {
            out.push_str(&format!("{}{r}\n", pad("Received:")));
        }
        out.push_str(&format!("{}{}ms", pad("Timeout:"), self.timeout_ms));
        if self.not_found {
            out.push_str("\nError: element(s) not found");
        }
        out
    }
}

/// Playwright's polling schedule for assertions: 100, 250, 500, then 1000 ms.
pub fn poll_interval_ms(attempt: usize) -> u64 {
    [100, 250, 500, 1000].get(attempt).copied().unwrap_or(1000)
}

/// Playwright's default assertion timeout.
pub const DEFAULT_EXPECT_TIMEOUT_MS: u64 = 5000;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::selectors::TextMatch;
    use serde_json::json;

    #[test]
    fn expected_text_like_playwright() {
        assert_eq!(
            expected_text(&TextMatch::Text("a".into()), TextOptions::HAVE),
            json!({"string": "a", "matchSubstring": false, "normalizeWhiteSpace": true})
        );
        assert_eq!(
            expected_text(&TextMatch::Text("a".into()), TextOptions::CONTAIN),
            json!({"string": "a", "matchSubstring": true, "normalizeWhiteSpace": true})
        );
        assert_eq!(
            expected_text(
                &TextMatch::Regex {
                    pattern: "^a.*".into(),
                    flags: "i".into()
                },
                TextOptions::EXACT
            ),
            json!({"regexSource": "^a.*", "regexFlags": "i", "matchSubstring": false, "normalizeWhiteSpace": false})
        );
        let ci = TextOptions {
            ignore_case: Some(true),
            ..TextOptions::HAVE
        };
        assert_eq!(
            expected_text(&TextMatch::Text("a".into()), ci)["ignoreCase"],
            json!(true)
        );
    }

    #[test]
    fn expected_display() {
        assert_eq!(
            display_expected(&TextMatch::Text("nope".into())),
            "\"nope\""
        );
        assert_eq!(
            display_expected(&TextMatch::Regex {
                pattern: "a+".into(),
                flags: "i".into()
            }),
            "/a+/i"
        );
    }

    #[test]
    fn failure_messages_like_playwright() {
        let m = Failure {
            subject: Subject::Locator("locator('#a')".into()),
            matcher: "toHaveText(expected)".into(),
            is_not: false,
            expected: "\"nope\"".into(),
            received: Some("\"Hello  world\"".into()),
            timeout_ms: 300,
            not_found: false,
        };
        assert_eq!(
            m.message(),
            "expect(locator).toHaveText(expected) failed\n\nLocator:  locator('#a')\nExpected: \"nope\"\nReceived: \"Hello  world\"\nTimeout:  300ms"
        );
        let missing = Failure {
            subject: Subject::Locator("locator('#zz')".into()),
            matcher: "toBeVisible()".into(),
            is_not: false,
            expected: "visible".into(),
            received: None,
            timeout_ms: 300,
            not_found: true,
        };
        assert_eq!(
            missing.message(),
            "expect(locator).toBeVisible() failed\n\nLocator: locator('#zz')\nExpected: visible\nTimeout: 300ms\nError: element(s) not found"
        );
        let not = Failure {
            subject: Subject::Locator("locator('#a')".into()),
            matcher: "toBeVisible()".into(),
            is_not: true,
            expected: "not visible".into(),
            received: Some("visible".into()),
            timeout_ms: 300,
            not_found: false,
        };
        assert!(
            not.message()
                .starts_with("expect(locator).not.toBeVisible() failed")
        );
        let page = Failure {
            subject: Subject::Page,
            matcher: "toHaveTitle(expected)".into(),
            is_not: false,
            expected: "\"t\"".into(),
            received: Some("\"\"".into()),
            timeout_ms: 300,
            not_found: false,
        };
        assert_eq!(
            page.message(),
            "expect(page).toHaveTitle(expected) failed\n\nExpected: \"t\"\nReceived: \"\"\nTimeout:  300ms"
        );
    }

    #[test]
    fn retry_schedule() {
        assert_eq!(
            (0..6).map(poll_interval_ms).collect::<Vec<_>>(),
            [100, 250, 500, 1000, 1000, 1000]
        );
    }
}
