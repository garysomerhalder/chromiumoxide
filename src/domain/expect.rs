//! Web-first assertions: the expected-value encoding Playwright's injected
//! `expect()` takes, and Playwright's failure messages.

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
