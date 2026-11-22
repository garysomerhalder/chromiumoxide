//! Playwright's `asLocator('javascript', selector)` (`locatorGenerators.ts`
//! with its selector parser): the locator code a selector reads as, e.g.
//! `getByRole('button', { name: 'Go' })`.

/// The JavaScript locator for `selector`, or the selector itself when it does
/// not parse (as Playwright does).
pub fn as_locator(selector: &str) -> String {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Selectors built by Playwright 1.63's own locators, with its asLocator.
    const CASES: &str = include_str!("data/as_locator_cases.json");

    #[test]
    fn matches_playwright() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(CASES).unwrap();
        for c in cases {
            let selector = c["selector"].as_str().unwrap();
            assert_eq!(
                as_locator(selector),
                c["locator"].as_str().unwrap(),
                "{selector}"
            );
        }
    }
}
