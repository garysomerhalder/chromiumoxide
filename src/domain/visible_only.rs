//! Playwright 1.66's `locateOnlyVisible` context option: the selector a
//! locator queries with gets `visible=true` at the end and before each
//! `nth=` (`first()`, `last()`, `nth()`), unless that position already has
//! an explicit visibility filter. Mirrors `restrictToVisible` in Playwright's
//! client `locator.ts`, on its selector parser (`selectorParser.ts`).

#[cfg(test)]
mod tests {
    use super::restrict_to_visible;

    /// Recorded with the Playwright 1.66.0 alpha of 2026-11-22
    /// (`locator.selectorForQuery()` with `locateOnlyVisible: true`).
    const PLAYWRIGHT_1_66: &[(&str, &str)] = &[
        ("div", "div >> visible=true"),
        ("div >> nth=0", "div >> visible=true >> nth=0"),
        ("div >> nth=-1", "div >> visible=true >> nth=-1"),
        ("div >> nth=1", "div >> visible=true >> nth=1"),
        (
            "div >> visible=false >> nth=0",
            "div >> visible=false >> nth=0",
        ),
        ("div >> visible=true", "div >> visible=true"),
        (
            "section >> internal:role=button",
            "section >> internal:role=button >> visible=true",
        ),
        (
            "section >> internal:has=\"internal:role=button[name=\\\"Go\\\"i]\"",
            "section >> internal:has=\"internal:role=button[name=\\\"Go\\\"i]\" >> visible=true",
        ),
        (
            "section >> internal:has-not=\"b\"",
            "section >> internal:has-not=\"b\" >> visible=true",
        ),
        (
            "div >> internal:and=\"internal:text=\\\"Two\\\"i\"",
            "div >> internal:and=\"internal:text=\\\"Two\\\"i\" >> visible=true",
        ),
        (
            "div >> internal:or=\"span\"",
            "div >> internal:or=\"span\" >> visible=true",
        ),
        (
            "internal:text=\"Two\"i",
            "internal:text=\"Two\"i >> visible=true",
        ),
        (
            "div >> internal:describe=\"the divs\"",
            "div >> internal:describe=\"the divs\" >> visible=true",
        ),
        (
            "div >> nth=0 >> internal:describe=\"first div\"",
            "div >> visible=true >> nth=0 >> internal:describe=\"first div\"",
        ),
        (
            "iframe >> internal:control=enter-frame >> p",
            "iframe >> internal:control=enter-frame >> p >> visible=true",
        ),
        (
            "iframe >> internal:control=enter-frame >> p >> nth=0",
            "iframe >> internal:control=enter-frame >> p >> visible=true >> nth=0",
        ),
        (
            "section >> internal:has=\"button >> nth=0\"",
            "section >> internal:has=\"button >> visible=true >> nth=0\" >> visible=true",
        ),
        (
            "*css=section >> button",
            "*css=section >> button >> visible=true",
        ),
        ("button:near(div)", "button:near(div) >> visible=true"),
        ("//div", "//div >> visible=true"),
        ("text=\"a >> b\"", "text=\"a >> b\" >> visible=true"),
        (
            "button >> left-of=\"div\", 10",
            "button >> left-of=\"div\", 10 >> visible=true",
        ),
    ];

    #[test]
    fn matches_playwright_1_66() {
        for (selector, expected) in PLAYWRIGHT_1_66 {
            assert_eq!(restrict_to_visible(selector), *expected, "{selector}");
        }
    }
}
