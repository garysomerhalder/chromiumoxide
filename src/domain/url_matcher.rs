//! URL matching like Playwright's `urlMatches` (glob strings and regexes).

use regex::Regex;

/// What a URL must match (`page.waitForURL`, `page.route`, ...).
#[derive(Debug, Clone)]
pub enum UrlMatcher {
    /// Playwright glob: `*` (no `/`), `**` (anything), `{a,b}`, `\` escapes.
    Glob {
        glob: String,
        regex: Regex,
    },
    Regex(Regex),
}

impl UrlMatcher {
    /// A Playwright glob pattern.
    pub fn glob(_glob: &str) -> Self {
        unimplemented!()
    }

    /// An exact URL (no glob characters are special).
    pub fn exact(_url: &str) -> Self {
        unimplemented!()
    }

    /// A regular expression (searched anywhere in the URL, like JS `RegExp.test`).
    pub fn regex(_pattern: &str) -> Result<Self, regex::Error> {
        unimplemented!()
    }

    pub fn matches(&self, _url: &str) -> bool {
        unimplemented!()
    }
}

/// Playwright's `globToRegexPattern`.
pub fn glob_to_regex_pattern(_glob: &str) -> Result<String, String> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(glob: &str, url: &str) -> bool {
        UrlMatcher::glob(glob).matches(url)
    }

    #[test]
    fn patterns_like_playwright() {
        assert_eq!(
            glob_to_regex_pattern("**/*.js").unwrap(),
            r"^(.*/)([^/]*)\.js$"
        );
        assert_eq!(
            glob_to_regex_pattern("foo/**/bar").unwrap(),
            r"^foo/((.+/)|)bar$"
        );
        assert_eq!(glob_to_regex_pattern("{a,b}?.c").unwrap(), r"^(a|b)\?\.c$");
        assert_eq!(glob_to_regex_pattern(r"\*x,y").unwrap(), r"^\*x\,y$");
        assert!(
            glob_to_regex_pattern("{a")
                .unwrap_err()
                .contains("unmatched '{'")
        );
        assert!(
            glob_to_regex_pattern("a}")
                .unwrap_err()
                .contains("unmatched '}'")
        );
        assert!(
            glob_to_regex_pattern("{a{b}}")
                .unwrap_err()
                .contains("nested '{'")
        );
    }

    #[test]
    fn glob_matching() {
        assert!(m("**/*.js", "https://localhost:8080/foo.js"));
        assert!(!m("**/*.css", "https://localhost:8080/foo.js"));
        assert!(m("*.js", "foo.js"));
        assert!(!m("*.js", "https://localhost:8080/foo.js"));
        assert!(m(
            "http://localhost:8080/?imple=foo",
            "http://localhost:8080/?imple=foo"
        ));
        assert!(!m(
            "http://localhost:8080/?imple=foo",
            "http://localhost:8080/Aimple=foo"
        ));
        assert!(m("**/foo/**", "https://localhost:8080/foo/bar/baz.js"));
        assert!(m("**/{a,b}.js", "https://x/b.js"));
        assert!(m("**/done?*", "http://127.0.0.1:1/done?x=1"));
    }

    #[test]
    fn exact_and_regex() {
        assert!(UrlMatcher::exact("http://a/b?c=*").matches("http://a/b?c=*"));
        assert!(!UrlMatcher::exact("http://a/b?c=*").matches("http://a/b?c=1"));
        let r = UrlMatcher::regex(r"/done\?x=\d$").unwrap();
        assert!(r.matches("http://h/done?x=1"));
        assert!(!r.matches("http://h/done?x=1/"));
    }
}
