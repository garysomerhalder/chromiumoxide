//! Source locations of API calls (Playwright's `metadata.location`), used by
//! `debugger.runTo(location)` and `debugger.pausedDetails().location`.

/// A place in the caller's source code.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceLocation {
    /// The file path (for `runTo`, any part of it, e.g. `main.rs`).
    pub file: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

impl SourceLocation {
    /// Any line of `file`.
    pub fn file(file: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            ..Default::default()
        }
    }

    /// `file:line`.
    pub fn line(file: impl Into<String>, line: u32) -> Self {
        Self {
            line: Some(line),
            ..Self::file(file)
        }
    }

    /// Playwright's `matchesLocation`: `self` is the call site, `target` the
    /// `runTo` argument (a part of the path, then optional line and column).
    pub fn matches(&self, target: &SourceLocation) -> bool {
        self.file.contains(&target.file)
            && target.line.is_none_or(|l| self.line == Some(l))
            && target.column.is_none_or(|c| self.column == Some(c))
    }
}

/// The `at file:line:column` entries of a `std::backtrace::Backtrace`
/// printout, innermost first.
pub fn backtrace_locations(text: &str) -> Vec<SourceLocation> {
    text.lines()
        .filter_map(|l| l.trim_start().strip_prefix("at "))
        .filter_map(parse_location)
        .collect()
}

fn parse_location(s: &str) -> Option<SourceLocation> {
    let (rest, last) = s.rsplit_once(':')?;
    let last: u32 = last.parse().ok()?;
    if let Some((file, line)) = rest.rsplit_once(':')
        && let Ok(line) = line.parse()
    {
        return Some(SourceLocation {
            file: file.to_owned(),
            line: Some(line),
            column: Some(last),
        });
    }
    Some(SourceLocation {
        file: rest.to_owned(),
        line: Some(last),
        column: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_follows_playwright() {
        let site = SourceLocation {
            file: "/w/tests/main.rs".into(),
            line: Some(10),
            column: Some(5),
        };
        assert!(site.matches(&SourceLocation::file("main.rs")));
        assert!(site.matches(&SourceLocation::line("tests/main.rs", 10)));
        assert!(!site.matches(&SourceLocation::line("main.rs", 11)));
        assert!(!site.matches(&SourceLocation::file("lib.rs")));
        assert!(site.matches(&SourceLocation {
            column: Some(5),
            ..SourceLocation::line("main.rs", 10)
        }));
        assert!(!site.matches(&SourceLocation {
            column: Some(6),
            ..SourceLocation::line("main.rs", 10)
        }));
    }

    #[test]
    fn parses_std_backtrace_output() {
        let text = "   0: std::backtrace::Backtrace::force_capture\n\
             at /rustc/abc/library/std/src/backtrace.rs:312:13\n\
           1: app::run::{{closure}}\n\
             at ./src/main.rs:7:9\n\
           2: <unknown>\n\
           3: C:\\w\\x.rs frame\n\
             at C:\\w\\x.rs:3\n";
        assert_eq!(
            backtrace_locations(text),
            [
                SourceLocation {
                    file: "/rustc/abc/library/std/src/backtrace.rs".into(),
                    line: Some(312),
                    column: Some(13)
                },
                SourceLocation {
                    file: "./src/main.rs".into(),
                    line: Some(7),
                    column: Some(9)
                },
                SourceLocation::line("C:\\w\\x.rs", 3),
            ]
        );
    }
}
