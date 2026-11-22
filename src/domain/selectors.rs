//! Playwright selector strings: `getBy*` builders and locator composition
//! (`isomorphic/locatorUtils.ts`, `client/locator.ts`). The strings are
//! evaluated in the page by Playwright's injected selector engines.

/// Text to match: a string (substring, case-insensitive unless exact) or a
/// JavaScript regular expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextMatch {
    Text(String),
    /// `/pattern/flags`.
    Regex {
        pattern: String,
        flags: String,
    },
}

impl TextMatch {
    pub fn regex(pattern: &str, flags: &str) -> Self {
        TextMatch::Regex {
            pattern: pattern.to_owned(),
            flags: flags.to_owned(),
        }
    }
}

impl From<&str> for TextMatch {
    fn from(s: &str) -> Self {
        TextMatch::Text(s.to_owned())
    }
}

impl From<String> for TextMatch {
    fn from(s: String) -> Self {
        TextMatch::Text(s)
    }
}

/// `getByRole` options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoleOptions {
    pub checked: Option<bool>,
    pub disabled: Option<bool>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    pub include_hidden: Option<bool>,
    pub level: Option<u32>,
    pub name: Option<TextMatch>,
    pub description: Option<TextMatch>,
    pub exact: bool,
    pub pressed: Option<bool>,
}

/// Playwright's `escapeForTextSelector`.
pub fn escape_for_text_selector(_t: &TextMatch, _exact: bool) -> String {
    unimplemented!()
}

/// Playwright's `escapeForAttributeSelector`.
pub fn escape_for_attribute_selector(_t: &TextMatch, _exact: bool) -> String {
    unimplemented!()
}

pub fn get_by_text(_t: &TextMatch, _exact: bool) -> String {
    unimplemented!()
}
pub fn get_by_label(_t: &TextMatch, _exact: bool) -> String {
    unimplemented!()
}
pub fn get_by_alt_text(_t: &TextMatch, _exact: bool) -> String {
    unimplemented!()
}
pub fn get_by_title(_t: &TextMatch, _exact: bool) -> String {
    unimplemented!()
}
pub fn get_by_placeholder(_t: &TextMatch, _exact: bool) -> String {
    unimplemented!()
}
pub fn get_by_test_id(_attribute: &str, _id: &TextMatch) -> String {
    unimplemented!()
}
pub fn get_by_role(_role: &str, _o: &RoleOptions) -> String {
    unimplemented!()
}

/// `locator.filter(...)` / constructor options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub has_text: Option<TextMatch>,
    pub has_not_text: Option<TextMatch>,
    /// Selector of a locator that must match inside.
    pub has: Option<String>,
    pub has_not: Option<String>,
    pub visible: Option<bool>,
}

/// Selector composition, as done by Playwright's client `Locator`.
pub mod compose {
    use super::*;

    /// `locator.locator("css")`.
    pub fn chain(_parent: &str, _child: &str) -> String {
        unimplemented!()
    }
    /// `locator.locator(otherLocator)`.
    pub fn chain_locator(_parent: &str, _child_selector: &str) -> String {
        unimplemented!()
    }
    pub fn nth(_selector: &str, _index: i64) -> String {
        unimplemented!()
    }
    pub fn filter(_selector: &str, _f: &Filter) -> String {
        unimplemented!()
    }
    pub fn and(_a: &str, _b: &str) -> String {
        unimplemented!()
    }
    pub fn or(_a: &str, _b: &str) -> String {
        unimplemented!()
    }
    /// `frameLocator` / `locator.contentFrame()`: following selectors apply inside the iframe.
    pub fn enter_frame(_selector: &str) -> String {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::compose::*;
    use super::*;

    fn t(s: &str) -> TextMatch {
        s.into()
    }

    #[test]
    fn get_by_builders_match_playwright() {
        assert_eq!(get_by_text(&t("Hello"), false), r#"internal:text="Hello"i"#);
        assert_eq!(get_by_text(&t("Hello"), true), r#"internal:text="Hello"s"#);
        assert_eq!(
            get_by_label(&t("Email"), false),
            r#"internal:label="Email"i"#
        );
        assert_eq!(
            get_by_placeholder(&t(r"a\b"), false),
            r#"internal:attr=[placeholder="a\\b"i]"#
        );
        assert_eq!(
            get_by_alt_text(&t(r#"say "hi""#), true),
            r#"internal:attr=[alt="say \"hi\""s]"#
        );
        assert_eq!(
            get_by_title(&t("T"), false),
            r#"internal:attr=[title="T"i]"#
        );
        assert_eq!(
            get_by_test_id("data-testid", &t("x")),
            r#"internal:testid=[data-testid="x"s]"#
        );
        assert_eq!(
            get_by_test_id("a,b", &t("x")),
            r#"internal:testid=["a,b"="x"s]"#
        );
    }

    #[test]
    fn get_by_role_options_in_playwright_order() {
        assert_eq!(
            get_by_role("button", &RoleOptions::default()),
            "internal:role=button"
        );
        let o = RoleOptions {
            name: Some(t("Submit")),
            ..Default::default()
        };
        assert_eq!(
            get_by_role("button", &o),
            r#"internal:role=button[name="Submit"i]"#
        );
        let o = RoleOptions {
            pressed: Some(true),
            level: Some(2),
            checked: Some(false),
            name: Some(t(r#"A "b""#)),
            exact: true,
            include_hidden: Some(true),
            ..Default::default()
        };
        assert_eq!(
            get_by_role("heading", &o),
            r#"internal:role=heading[checked=false][include-hidden=true][level=2][name="A \"b\""s][pressed=true]"#
        );
    }

    #[test]
    fn regexes_are_escaped_like_playwright() {
        assert_eq!(
            get_by_text(&TextMatch::regex(r#"hel"lo"#, "i"), false),
            r#"internal:text=/hel\"lo/i"#
        );
        assert_eq!(
            get_by_text(&TextMatch::regex(r#"a\"b"#, ""), false),
            r#"internal:text=/a\"b/"#
        );
        assert_eq!(
            get_by_text(&TextMatch::regex("a>>b", ""), false),
            r"internal:text=/a\>\>b/"
        );
        assert_eq!(
            get_by_text(&TextMatch::regex(r#"""#, ""), false),
            r#"internal:text=/\""/"#
        );
    }

    #[test]
    fn composition_matches_playwright_client() {
        assert_eq!(chain("div", "span"), "div >> span");
        assert_eq!(
            chain_locator("div", "internal:text=\"x\"i"),
            r#"div >> internal:chain="internal:text=\"x\"i""#
        );
        assert_eq!(nth("li", 2), "li >> nth=2");
        assert_eq!(nth("li", -1), "li >> nth=-1");
        let f = Filter {
            has_text: Some(t("a")),
            has_not_text: Some(t("b")),
            has: Some("span".into()),
            has_not: Some("em".into()),
            visible: Some(true),
        };
        assert_eq!(
            filter("li", &f),
            r#"li >> internal:has-text="a"i >> internal:has-not-text="b"i >> internal:has="span" >> internal:has-not="em" >> visible=true"#
        );
        assert_eq!(and("a", "b"), r#"a >> internal:and="b""#);
        assert_eq!(or("a", "b"), r#"a >> internal:or="b""#);
        assert_eq!(
            enter_frame("iframe"),
            "iframe >> internal:control=enter-frame"
        );
    }
}
