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

fn json(s: &str) -> String {
    serde_json::to_string(s).expect("string serialises")
}

/// Playwright's `escapeRegexForSelector`.
fn escape_regex(pattern: &str, flags: &str) -> String {
    let source = format!("/{pattern}/{flags}");
    if flags.contains('u') || flags.contains('v') {
        return source;
    }
    static QUOTES: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let quotes =
        QUOTES.get_or_init(|| regex::Regex::new(r#"(^|[^\\])(\\\\)*(["'`])"#).expect("valid"));
    quotes
        .replace_all(&source, r"$1$2\$3")
        .replace(">>", r"\>\>")
}

/// Playwright's `escapeForTextSelector`.
pub fn escape_for_text_selector(t: &TextMatch, exact: bool) -> String {
    match t {
        TextMatch::Text(s) => format!("{}{}", json(s), if exact { "s" } else { "i" }),
        TextMatch::Regex { pattern, flags } => escape_regex(pattern, flags),
    }
}

/// Playwright's `escapeForAttributeSelector`.
pub fn escape_for_attribute_selector(t: &TextMatch, exact: bool) -> String {
    match t {
        TextMatch::Text(s) => format!(
            "\"{}\"{}",
            s.replace('\\', "\\\\").replace('"', "\\\""),
            if exact { "s" } else { "i" }
        ),
        TextMatch::Regex { pattern, flags } => escape_regex(pattern, flags),
    }
}

fn by_attribute(name: &str, t: &TextMatch, exact: bool) -> String {
    format!(
        "internal:attr=[{name}={}]",
        escape_for_attribute_selector(t, exact)
    )
}

pub fn get_by_text(t: &TextMatch, exact: bool) -> String {
    format!("internal:text={}", escape_for_text_selector(t, exact))
}
pub fn get_by_label(t: &TextMatch, exact: bool) -> String {
    format!("internal:label={}", escape_for_text_selector(t, exact))
}
pub fn get_by_alt_text(t: &TextMatch, exact: bool) -> String {
    by_attribute("alt", t, exact)
}
pub fn get_by_title(t: &TextMatch, exact: bool) -> String {
    by_attribute("title", t, exact)
}
pub fn get_by_placeholder(t: &TextMatch, exact: bool) -> String {
    by_attribute("placeholder", t, exact)
}
pub fn get_by_test_id(attribute: &str, id: &TextMatch) -> String {
    let attr = if attribute.contains(',') {
        json(attribute)
    } else {
        attribute.to_owned()
    };
    format!(
        "internal:testid=[{attr}={}]",
        escape_for_attribute_selector(id, true)
    )
}
pub fn get_by_role(role: &str, o: &RoleOptions) -> String {
    let mut props: Vec<(&str, String)> = Vec::new();
    let mut flag = |name: &'static str, v: Option<bool>| {
        if let Some(v) = v {
            props.push((name, v.to_string()));
        }
    };
    flag("checked", o.checked);
    flag("disabled", o.disabled);
    flag("selected", o.selected);
    flag("expanded", o.expanded);
    flag("include-hidden", o.include_hidden);
    if let Some(level) = o.level {
        props.push(("level", level.to_string()));
    }
    if let Some(name) = &o.name {
        props.push(("name", escape_for_attribute_selector(name, o.exact)));
    }
    if let Some(d) = &o.description {
        props.push(("description", escape_for_attribute_selector(d, o.exact)));
    }
    if let Some(p) = o.pressed {
        props.push(("pressed", p.to_string()));
    }
    let props: String = props.iter().map(|(n, v)| format!("[{n}={v}]")).collect();
    format!("internal:role={role}{props}")
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
    pub fn chain(parent: &str, child: &str) -> String {
        format!("{parent} >> {child}")
    }
    /// `locator.locator(otherLocator)`.
    pub fn chain_locator(parent: &str, child_selector: &str) -> String {
        format!("{parent} >> internal:chain={}", json(child_selector))
    }
    pub fn nth(selector: &str, index: i64) -> String {
        format!("{selector} >> nth={index}")
    }
    pub fn filter(selector: &str, f: &Filter) -> String {
        let mut s = selector.to_owned();
        if let Some(t) = &f.has_text {
            s += &format!(
                " >> internal:has-text={}",
                escape_for_text_selector(t, false)
            );
        }
        if let Some(t) = &f.has_not_text {
            s += &format!(
                " >> internal:has-not-text={}",
                escape_for_text_selector(t, false)
            );
        }
        if let Some(h) = &f.has {
            s += &format!(" >> internal:has={}", json(h));
        }
        if let Some(h) = &f.has_not {
            s += &format!(" >> internal:has-not={}", json(h));
        }
        if let Some(v) = f.visible {
            s += &format!(" >> visible={v}");
        }
        s
    }
    pub fn and(a: &str, b: &str) -> String {
        format!("{a} >> internal:and={}", json(b))
    }
    pub fn or(a: &str, b: &str) -> String {
        format!("{a} >> internal:or={}", json(b))
    }
    /// `frameLocator` / `locator.contentFrame()`: following selectors apply inside the iframe.
    pub fn enter_frame(selector: &str) -> String {
        format!("{selector} >> internal:control=enter-frame")
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
            get_by_text(&TextMatch::regex(r#""""#, ""), false),
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
