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

/// A backtrace frame's file as an absolute path: Rust prints files under
/// the working directory relative to it (`./a.rs` on Unix, `.\a.rs` or
/// `a.rs` on Windows); those are joined to `cwd` (with `/` separators).
pub fn resolve_frame_file(file: &str, cwd: Option<&std::path::Path>) -> String {
    let absolute =
        file.starts_with('/') || file.starts_with("\\\\") || file.as_bytes().get(1) == Some(&b':');
    match cwd {
        Some(cwd) if !absolute => {
            let rel = file
                .strip_prefix("./")
                .or_else(|| file.strip_prefix(".\\"))
                .unwrap_or(file)
                .replace('\\', "/");
            // Joined by hand: `Path::join` would put `\` before `rel` on
            // Windows.
            let cwd = cwd.to_string_lossy();
            format!("{}/{rel}", cwd.trim_end_matches(['/', '\\']))
        }
        _ => file.to_owned(),
    }
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
    fn relative_frames_are_resolved_against_the_working_directory() {
        let cwd = std::path::Path::new("/w");
        assert_eq!(
            resolve_frame_file("./tests/a.rs", Some(cwd)),
            "/w/tests/a.rs"
        );
        assert_eq!(
            resolve_frame_file(".\\tests\\a.rs", Some(cwd)),
            "/w/tests/a.rs"
        );
        assert_eq!(
            resolve_frame_file("tests\\a.rs", Some(cwd)),
            "/w/tests/a.rs"
        );
        assert_eq!(resolve_frame_file("tests/a.rs", Some(cwd)), "/w/tests/a.rs");
        // Absolute paths stay as they are.
        assert_eq!(
            resolve_frame_file("/rustc/x/lib.rs", Some(cwd)),
            "/rustc/x/lib.rs"
        );
        assert_eq!(resolve_frame_file("C:\\u\\a.rs", Some(cwd)), "C:\\u\\a.rs");
        assert_eq!(
            resolve_frame_file("\\\\server\\a.rs", Some(cwd)),
            "\\\\server\\a.rs"
        );
        assert_eq!(resolve_frame_file("./a.rs", None), "./a.rs");
    }

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
