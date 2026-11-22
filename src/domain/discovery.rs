//! Browser discovery rules, in Playwright's order.
//!
//! This module only *describes* where to look ([`probes`]) and how to rank what
//! is found ([`rank_entries`]); the application layer walks a file system port.

use std::path::PathBuf;

/// Operating system family, for layout and system-install rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Os {
    #[default]
    Linux,
    MacOs,
    Windows,
}

/// A snapshot of the environment relevant to discovery.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiscoveryEnv {
    pub os: Os,
    /// `LaunchOptions::executable_path`.
    pub explicit: Option<PathBuf>,
    /// `MOCKUP_CHROMIUM`.
    pub mockup_chromium: Option<PathBuf>,
    /// `CHROME`.
    pub chrome: Option<PathBuf>,
    /// `PLAYWRIGHT_BROWSERS_PATH`.
    pub playwright_browsers_path: Option<PathBuf>,
    /// The user's home directory.
    pub home: Option<PathBuf>,
    /// Entries of `PATH`.
    pub path: Vec<PathBuf>,
    /// `%ProgramFiles%`, `%ProgramFiles(x86)%` (Windows).
    pub program_files: Vec<PathBuf>,
    /// `%LOCALAPPDATA%` (Windows).
    pub local_app_data: Option<PathBuf>,
}

/// Where a discovered executable came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Explicit,
    MockupChromiumEnv,
    ChromeEnv,
    PlaywrightBrowsersPath,
    OptPwBrowsers,
    PlaywrightCache,
    ChromeForTesting,
    System,
}

/// How a browsers directory is organised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `chromium_headless_shell-<rev>/…`, `chromium-<rev>/…`.
    Playwright,
    /// `<platform>-<version>/…` (chromiumoxide fetcher, puppeteer caches).
    ChromeForTesting,
}

/// One place to look, in priority order.
#[derive(Debug, Clone, PartialEq)]
pub enum Probe {
    /// Must exist: discovery fails instead of falling back (explicit path, env vars).
    Required { path: PathBuf, source: Source },
    /// A browsers directory to scan.
    Root {
        root: PathBuf,
        layout: Layout,
        source: Source,
    },
    /// A single well-known location; skipped when missing.
    Candidate { path: PathBuf, source: Source },
}

/// All probes for `env`, highest priority first.
pub fn probes(_env: &DiscoveryEnv) -> Vec<Probe> {
    unimplemented!()
}

/// Ranks directory entries of a browsers root: for the Playwright layout,
/// `chromium_headless_shell-*` before `chromium-*`, each newest revision first;
/// for Chrome for Testing, newest version first. Unrelated entries are dropped.
pub fn rank_entries(_layout: Layout, _names: &[String]) -> Vec<String> {
    unimplemented!()
}

/// Executable paths relative to one revision directory, preferred first.
pub fn executables_within(_os: Os) -> Vec<PathBuf> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn linux_env() -> DiscoveryEnv {
        DiscoveryEnv {
            os: Os::Linux,
            explicit: Some(p("/my/chrome")),
            mockup_chromium: Some(p("/mock/chrome")),
            chrome: Some(p("/env/chrome")),
            playwright_browsers_path: Some(p("/pwb")),
            home: Some(p("/home/u")),
            path: vec![p("/usr/local/bin"), p("/usr/bin")],
            ..Default::default()
        }
    }

    #[test]
    fn order_follows_playwright_then_cft_then_system() {
        let got = probes(&linux_env());
        let head = &got[..9];
        assert_eq!(
            head,
            &[
                Probe::Required {
                    path: p("/my/chrome"),
                    source: Source::Explicit
                },
                Probe::Required {
                    path: p("/mock/chrome"),
                    source: Source::MockupChromiumEnv
                },
                Probe::Required {
                    path: p("/env/chrome"),
                    source: Source::ChromeEnv
                },
                Probe::Root {
                    root: p("/pwb"),
                    layout: Layout::Playwright,
                    source: Source::PlaywrightBrowsersPath
                },
                Probe::Root {
                    root: p("/opt/pw-browsers"),
                    layout: Layout::Playwright,
                    source: Source::OptPwBrowsers
                },
                Probe::Root {
                    root: p("/home/u/.cache/ms-playwright"),
                    layout: Layout::Playwright,
                    source: Source::PlaywrightCache
                },
                Probe::Root {
                    root: p("/home/u/Library/Caches/ms-playwright"),
                    layout: Layout::Playwright,
                    source: Source::PlaywrightCache
                },
                Probe::Root {
                    root: p("/home/u/.cache/chromiumoxide"),
                    layout: Layout::ChromeForTesting,
                    source: Source::ChromeForTesting
                },
                Probe::Root {
                    root: p("/home/u/.cache/puppeteer/chrome-headless-shell"),
                    layout: Layout::ChromeForTesting,
                    source: Source::ChromeForTesting
                },
            ]
        );
        assert_eq!(
            got[9],
            Probe::Root {
                root: p("/home/u/.cache/puppeteer/chrome"),
                layout: Layout::ChromeForTesting,
                source: Source::ChromeForTesting
            }
        );
        let system: Vec<_> = got[10..].iter().cloned().collect();
        assert_eq!(
            system[0],
            Probe::Candidate {
                path: p("/usr/local/bin/google-chrome-stable"),
                source: Source::System
            }
        );
        assert!(system.contains(&Probe::Candidate {
            path: p("/usr/bin/chromium"),
            source: Source::System
        }));
        assert!(system.contains(&Probe::Candidate {
            path: p("/opt/google/chrome/chrome"),
            source: Source::System
        }));
        assert!(system.iter().all(|pr| matches!(
            pr,
            Probe::Candidate {
                source: Source::System,
                ..
            }
        )));
    }

    #[test]
    fn unset_sources_are_skipped_and_pwb_zero_is_ignored() {
        let env = DiscoveryEnv {
            os: Os::Linux,
            playwright_browsers_path: Some(p("0")),
            ..Default::default()
        };
        let got = probes(&env);
        assert_eq!(
            got[0],
            Probe::Root {
                root: p("/opt/pw-browsers"),
                layout: Layout::Playwright,
                source: Source::OptPwBrowsers
            }
        );
        assert!(!got.iter().any(|pr| matches!(pr, Probe::Required { .. })));
    }

    #[test]
    fn macos_system_chrome_locations() {
        let env = DiscoveryEnv {
            os: Os::MacOs,
            home: Some(p("/Users/u")),
            ..Default::default()
        };
        let got = probes(&env);
        assert!(got.contains(&Probe::Candidate {
            path: p("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            source: Source::System
        }));
        assert!(got.contains(&Probe::Root {
            root: p("/Users/u/Library/Caches/chromiumoxide"),
            layout: Layout::ChromeForTesting,
            source: Source::ChromeForTesting
        }));
    }

    #[test]
    fn windows_system_chrome_locations() {
        let env = DiscoveryEnv {
            os: Os::Windows,
            program_files: vec![p("C:/Program Files")],
            local_app_data: Some(p("C:/Users/u/AppData/Local")),
            ..Default::default()
        };
        let got = probes(&env);
        assert!(got.contains(&Probe::Root {
            root: p("C:/Users/u/AppData/Local/ms-playwright"),
            layout: Layout::Playwright,
            source: Source::PlaywrightCache
        }));
        assert!(got.contains(&Probe::Candidate {
            path: p("C:/Program Files/Google/Chrome/Application/chrome.exe"),
            source: Source::System
        }));
    }

    #[test]
    fn playwright_entries_headless_shell_first_newest_first() {
        let got = rank_entries(
            Layout::Playwright,
            &names(&[
                "chromium-1200",
                "ffmpeg-1011",
                "chromium_headless_shell-999",
                "chromium-1243",
                "chromium_headless_shell-1243",
                "chromium_headless_shell",
                "chromium",
                ".links",
                "chromium-tip-of-tree-1300",
                "webkit-2000",
            ]),
        );
        assert_eq!(
            got,
            names(&[
                "chromium_headless_shell-1243",
                "chromium_headless_shell-999",
                "chromium-1243",
                "chromium-1200"
            ])
        );
    }

    #[test]
    fn cft_entries_newest_version_first() {
        let got = rank_entries(
            Layout::ChromeForTesting,
            &names(&[
                "linux-131.0.6778.85",
                "linux-140.0.7339.5",
                "linux-99.0.1.2",
                "notes.txt",
                "linux-131.0.6778.85.zip",
            ]),
        );
        assert_eq!(
            got,
            names(&[
                "linux-140.0.7339.5",
                "linux-131.0.6778.85",
                "linux-99.0.1.2"
            ])
        );
    }

    #[test]
    fn executables_prefer_headless_shell_layouts() {
        let linux = executables_within(Os::Linux);
        assert_eq!(
            linux[0],
            p("chrome-headless-shell-linux64/chrome-headless-shell")
        );
        assert!(linux.contains(&p("chrome-linux64/chrome")));
        assert!(linux.contains(&p("chrome-linux/chrome")));
        let mac = executables_within(Os::MacOs);
        assert!(mac.contains(&p("chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")));
        assert!(mac.contains(&p("chrome-mac/Chromium.app/Contents/MacOS/Chromium")));
        let win = executables_within(Os::Windows);
        assert_eq!(
            win[0],
            p("chrome-headless-shell-win64/chrome-headless-shell.exe")
        );
        assert!(win.contains(&p("chrome-win64/chrome.exe")));
    }
}
