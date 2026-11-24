//! Playwright's WebKit: command line, default user agent, context options
//! the engine cannot honour.

use std::path::Path;

/// The Safari version Playwright 1.65.1 reports for its WebKit build.
pub const BROWSER_VERSION: &str = "27.2";

/// Playwright's default WebKit user agent (a macOS Safari one on every
/// system, as in Playwright).
pub fn default_user_agent() -> String {
    format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{BROWSER_VERSION} Safari/605.1.15"
    )
}

/// Playwright's WebKit arguments: the inspector pipe, Windows' compositing
/// switch, `--headless`, then a persistent profile (and `about:blank`) or
/// no startup window, and the caller's arguments.
pub fn command_line(
    headless: bool,
    user_data_dir: Option<&Path>,
    args: &[String],
    windows: bool,
) -> Vec<String> {
    let mut out = vec!["--inspector-pipe".to_owned()];
    if windows {
        out.push("--disable-accelerated-compositing".to_owned());
    }
    if headless {
        out.push("--headless".to_owned());
    }
    match user_data_dir {
        Some(dir) => out.push(format!("--user-data-dir={}", dir.display())),
        None => out.push("--no-startup-window".to_owned()),
    }
    out.extend(args.iter().cloned());
    if user_data_dir.is_some() {
        out.push("about:blank".to_owned());
    }
    out
}

/// WebKit's `Input` modifier bits (Shift 1, Control 2, Alt 4, Meta 8) from
/// the CDP bits the shared keyboard model uses.
pub fn modifiers(cdp: u32) -> u32 {
    use crate::domain::input::{ALT, CONTROL, META, SHIFT};
    [(SHIFT, 1), (CONTROL, 2), (ALT, 4), (META, 8)]
        .iter()
        .filter(|(bit, _)| cdp & bit != 0)
        .map(|(_, wk)| wk)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playwright_arguments() {
        assert_eq!(
            command_line(true, None, &["--foo".into()], false),
            [
                "--inspector-pipe",
                "--headless",
                "--no-startup-window",
                "--foo"
            ]
        );
        assert_eq!(
            command_line(false, Some(Path::new("/p")), &[], true),
            [
                "--inspector-pipe",
                "--disable-accelerated-compositing",
                "--user-data-dir=/p",
                "about:blank"
            ]
        );
    }

    #[test]
    fn modifiers_use_webkits_bits() {
        use crate::domain::input::{ALT, CONTROL, META, SHIFT};
        assert_eq!(modifiers(SHIFT), 1);
        assert_eq!(modifiers(CONTROL), 2);
        assert_eq!(modifiers(ALT), 4);
        assert_eq!(modifiers(META), 8);
        assert_eq!(modifiers(SHIFT | META), 9);
        assert_eq!(modifiers(0), 0);
    }

    #[test]
    fn default_user_agent_is_playwrights() {
        assert_eq!(
            default_user_agent(),
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.2 Safari/605.1.15"
        );
    }
}
