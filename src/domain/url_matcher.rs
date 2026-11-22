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
    /// A Playwright glob pattern. Invalid globs (unbalanced braces) match
    /// nothing; use [`glob_to_regex_pattern`] to validate.
    pub fn glob(glob: &str) -> Self {
        let pattern = glob_to_regex_pattern(glob).unwrap_or_else(|_| "$^".to_owned());
        let regex = Regex::new(&pattern).unwrap_or_else(|_| Regex::new("$^").expect("valid"));
        UrlMatcher::Glob { glob: glob.to_owned(), regex }
    }

    /// An exact URL (no glob characters are special).
    pub fn exact(url: &str) -> Self {
        let regex = Regex::new(&format!("^{}$", regex::escape(url))).expect("escaped regex is valid");
        UrlMatcher::Glob { glob: url.to_owned(), regex }
    }

    /// A regular expression (searched anywhere in the URL, like JS `RegExp.test`).
    pub fn regex(pattern: &str) -> Result<Self, regex::Error> {
        Ok(UrlMatcher::Regex(Regex::new(pattern)?))
    }

    pub fn matches(&self, url: &str) -> bool {
        match self {
            UrlMatcher::Glob { regex, .. } | UrlMatcher::Regex(regex) => regex.is_match(url),
        }
    }
}

impl From<&str> for UrlMatcher {
    fn from(glob: &str) -> Self {
        UrlMatcher::glob(glob)
    }
}

impl From<Regex> for UrlMatcher {
    fn from(regex: Regex) -> Self {
        UrlMatcher::Regex(regex)
    }
}

const ESCAPED: &[char] = &['$', '^', '+', '.', '*', '(', ')', '|', '\\', '?', '{', '}', '[', ']'];

/// Playwright's `globToRegexPattern`.
pub fn glob_to_regex_pattern(glob: &str) -> Result<String, String> {
    let chars: Vec<char> = glob.chars().collect();
    let mut out = String::from("^");
    let mut in_group = false;
    let esc = |c: char| if ESCAPED.contains(&c) { format!("\\{c}") } else { c.to_string() };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            i += 1;
            out.push_str(&esc(chars[i]));
            i += 1;
            continue;
        }
        if c == '*' {
            let before = if i > 0 { Some(chars[i - 1]) } else { None };
            let mut stars = 1;
            while chars.get(i + 1) == Some(&'*') {
                stars += 1;
                i += 1;
            }
            if stars > 1 {
                if chars.get(i + 1) == Some(&'/') {
                    out.push_str(if before == Some('/') { "((.+/)|)" } else { "(.*/)" });
                    i += 1;
                } else {
                    out.push_str("(.*)");
                }
            } else {
                out.push_str("([^/]*)");
            }
            i += 1;
            continue;
        }
        match c {
            '{' => {
                if in_group {
                    return Err(format!("Invalid glob pattern {glob:?}: nested '{{' is not supported"));
                }
                in_group = true;
                out.push('(');
            }
            '}' => {
                if !in_group {
                    return Err(format!("Invalid glob pattern {glob:?}: unmatched '}}'"));
                }
                in_group = false;
                out.push(')');
            }
            ',' if in_group => out.push('|'),
            ',' => out.push_str("\\,"),
            _ => out.push_str(&esc(c)),
        }
        i += 1;
    }
    if in_group {
        return Err(format!("Invalid glob pattern {glob:?}: unmatched '{{'"));
    }
    out.push('$');
    Ok(out)
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
        assert!(m("**/a,b", "http://h/a,b"));
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
