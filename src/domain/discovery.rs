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
pub fn probes(env: &DiscoveryEnv) -> Vec<Probe> {
    let mut out = Vec::new();
    let required = [
        (&env.explicit, Source::Explicit),
        (&env.mockup_chromium, Source::MockupChromiumEnv),
        (&env.chrome, Source::ChromeEnv),
    ];
    for (path, source) in required {
        if let Some(path) = path.as_ref().filter(|p| !p.as_os_str().is_empty()) {
            out.push(Probe::Required {
                path: path.clone(),
                source,
            });
        }
    }

    let playwright = |root: PathBuf, source| Probe::Root {
        root,
        layout: Layout::Playwright,
        source,
    };
    if let Some(root) = env
        .playwright_browsers_path
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty() && p.as_os_str() != "0")
    {
        out.push(playwright(root.clone(), Source::PlaywrightBrowsersPath));
    }
    if env.os != Os::Windows {
        out.push(playwright(
            PathBuf::from("/opt/pw-browsers"),
            Source::OptPwBrowsers,
        ));
    }
    if let Some(home) = &env.home {
        out.push(playwright(
            home.join(".cache/ms-playwright"),
            Source::PlaywrightCache,
        ));
        out.push(playwright(
            home.join("Library/Caches/ms-playwright"),
            Source::PlaywrightCache,
        ));
    }
    if let Some(local) = &env.local_app_data {
        out.push(playwright(
            local.join("ms-playwright"),
            Source::PlaywrightCache,
        ));
    }

    let cft = |root: PathBuf| Probe::Root {
        root,
        layout: Layout::ChromeForTesting,
        source: Source::ChromeForTesting,
    };
    let fetcher_cache = match env.os {
        Os::Linux => env.home.as_ref().map(|h| h.join(".cache/chromiumoxide")),
        Os::MacOs => env
            .home
            .as_ref()
            .map(|h| h.join("Library/Caches/chromiumoxide")),
        Os::Windows => env.local_app_data.as_ref().map(|l| l.join("chromiumoxide")),
    };
    out.extend(fetcher_cache.map(cft));
    if let Some(home) = &env.home {
        out.push(cft(home.join(".cache/puppeteer/chrome-headless-shell")));
        out.push(cft(home.join(".cache/puppeteer/chrome")));
    }

    out.extend(
        system_candidates(env)
            .into_iter()
            .map(|path| Probe::Candidate {
                path,
                source: Source::System,
            }),
    );
    out
}

fn system_candidates(env: &DiscoveryEnv) -> Vec<PathBuf> {
    match env.os {
        Os::Linux => {
            const NAMES: &[&str] = &[
                "google-chrome-stable",
                "google-chrome",
                "chromium",
                "chromium-browser",
                "chrome",
            ];
            let mut out: Vec<PathBuf> = NAMES
                .iter()
                .flat_map(|name| env.path.iter().map(move |dir| dir.join(name)))
                .collect();
            out.extend(
                [
                    "/opt/google/chrome/chrome",
                    "/usr/bin/google-chrome",
                    "/usr/bin/chromium",
                    "/snap/bin/chromium",
                ]
                .into_iter()
                .map(PathBuf::from),
            );
            out
        }
        Os::MacOs => {
            let apps = [
                "Google Chrome.app/Contents/MacOS/Google Chrome",
                "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
                "Chromium.app/Contents/MacOS/Chromium",
            ];
            let mut out: Vec<PathBuf> = apps
                .iter()
                .map(|a| PathBuf::from("/Applications").join(a))
                .collect();
            if let Some(home) = &env.home {
                out.extend(apps.iter().map(|a| home.join("Applications").join(a)));
            }
            out
        }
        Os::Windows => {
            let rel = "Google/Chrome/Application/chrome.exe";
            let mut out: Vec<PathBuf> = env.program_files.iter().map(|d| d.join(rel)).collect();
            out.extend(env.local_app_data.iter().map(|d| d.join(rel)));
            out.extend(env.path.iter().map(|d| d.join("chrome.exe")));
            out
        }
    }
}

/// Ranks directory entries of a browsers root: for the Playwright layout,
/// `chromium_headless_shell-*` before `chromium-*`, each newest revision first;
/// for Chrome for Testing, newest version first. Unrelated entries are dropped.
pub fn rank_entries(layout: Layout, names: &[String]) -> Vec<String> {
    let mut ranked: Vec<((u8, Vec<u64>), &String)> = names
        .iter()
        .filter_map(|name| {
            let key = match layout {
                Layout::Playwright => {
                    let (class, rest) =
                        if let Some(rest) = name.strip_prefix("chromium_headless_shell-") {
                            (0, rest)
                        } else {
                            (1, name.strip_prefix("chromium-")?)
                        };
                    (class, vec![rest.parse::<u64>().ok()?])
                }
                Layout::ChromeForTesting => {
                    let (_, version) = name.rsplit_once('-')?;
                    let parts = version
                        .split('.')
                        .map(|part| part.parse::<u64>().ok())
                        .collect::<Option<Vec<_>>>()?;
                    (0, parts)
                }
            };
            Some((key, name))
        })
        .collect();
    // class ascending, version descending
    ranked.sort_by(|(a, _), (b, _)| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    ranked.into_iter().map(|(_, name)| name.clone()).collect()
}

/// Executable paths relative to one revision directory, preferred first.
pub fn executables_within(os: Os) -> Vec<PathBuf> {
    let rel: &[&str] = match os {
        Os::Linux => &[
            "chrome-headless-shell-linux64/chrome-headless-shell",
            "chrome-headless-shell-linux-arm64/chrome-headless-shell",
            "chrome-linux64/chrome",
            "chrome-linux/chrome",
            "chrome-linux-arm64/chrome",
        ],
        Os::MacOs => &[
            "chrome-headless-shell-mac-arm64/chrome-headless-shell",
            "chrome-headless-shell-mac-x64/chrome-headless-shell",
            "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            "chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            "chrome-mac-arm64/Chromium.app/Contents/MacOS/Chromium",
            "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
        ],
        Os::Windows => &[
            "chrome-headless-shell-win64/chrome-headless-shell.exe",
            "chrome-headless-shell-win32/chrome-headless-shell.exe",
            "chrome-win64/chrome.exe",
            "chrome-win/chrome.exe",
        ],
    };
    rel.iter().map(PathBuf::from).collect()
}

/// Where Playwright looks for a branded browser `channel` (`chrome`,
/// `chrome-beta`, `chrome-dev`, `chrome-canary`, `msedge`, `msedge-beta`,
/// `msedge-dev`, `msedge-canary`), in order. An unknown name, or a channel
/// that does not exist on this system (no canary on Linux), is an error.
pub fn channel_paths(channel: &str, env: &DiscoveryEnv) -> Result<Vec<PathBuf>, String> {
    let unsupported = || format!("Unsupported chromium channel \"{channel}\"");
    let (linux, mac, windows): (Option<&str>, &str, &str) = match channel {
        "chrome" => (
            Some("/opt/google/chrome/chrome"),
            "Google Chrome",
            "Google/Chrome/Application/chrome.exe",
        ),
        "chrome-beta" => (
            Some("/opt/google/chrome-beta/chrome"),
            "Google Chrome Beta",
            "Google/Chrome Beta/Application/chrome.exe",
        ),
        "chrome-dev" => (
            Some("/opt/google/chrome-unstable/chrome"),
            "Google Chrome Dev",
            "Google/Chrome Dev/Application/chrome.exe",
        ),
        "chrome-canary" => (
            None,
            "Google Chrome Canary",
            "Google/Chrome SxS/Application/chrome.exe",
        ),
        "msedge" => (
            Some("/opt/microsoft/msedge/msedge"),
            "Microsoft Edge",
            "Microsoft/Edge/Application/msedge.exe",
        ),
        "msedge-beta" => (
            Some("/opt/microsoft/msedge-beta/msedge"),
            "Microsoft Edge Beta",
            "Microsoft/Edge Beta/Application/msedge.exe",
        ),
        "msedge-dev" => (
            Some("/opt/microsoft/msedge-dev/msedge"),
            "Microsoft Edge Dev",
            "Microsoft/Edge Dev/Application/msedge.exe",
        ),
        "msedge-canary" => (
            None,
            "Microsoft Edge Canary",
            "Microsoft/Edge SxS/Application/msedge.exe",
        ),
        _ => return Err(unsupported()),
    };
    match env.os {
        Os::Linux => linux
            .map(|p| vec![PathBuf::from(p)])
            .ok_or_else(|| format!("Chromium distribution '{channel}' is not supported on Linux")),
        Os::MacOs => Ok(vec![PathBuf::from(format!(
            "/Applications/{mac}.app/Contents/MacOS/{mac}"
        ))]),
        Os::Windows => Ok(env
            .local_app_data
            .iter()
            .chain(&env.program_files)
            .map(|root| root.join(windows))
            .collect()),
    }
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

    #[test]
    fn channels_are_where_playwright_looks() {
        let linux = DiscoveryEnv {
            os: Os::Linux,
            ..Default::default()
        };
        assert_eq!(
            channel_paths("msedge", &linux).unwrap(),
            [p("/opt/microsoft/msedge/msedge")]
        );
        assert_eq!(
            channel_paths("chrome-beta", &linux).unwrap(),
            [p("/opt/google/chrome-beta/chrome")]
        );
        let mac = DiscoveryEnv {
            os: Os::MacOs,
            ..Default::default()
        };
        assert_eq!(
            channel_paths("msedge-canary", &mac).unwrap(),
            [p(
                "/Applications/Microsoft Edge Canary.app/Contents/MacOS/Microsoft Edge Canary"
            )]
        );
        assert_eq!(
            channel_paths("chrome", &mac).unwrap(),
            [p(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
            )]
        );
        let windows = DiscoveryEnv {
            os: Os::Windows,
            local_app_data: Some(p("C:/Users/u/AppData/Local")),
            program_files: vec![p("C:/Program Files"), p("C:/Program Files (x86)")],
            ..Default::default()
        };
        assert_eq!(
            channel_paths("msedge", &windows).unwrap(),
            [
                p("C:/Users/u/AppData/Local/Microsoft/Edge/Application/msedge.exe"),
                p("C:/Program Files/Microsoft/Edge/Application/msedge.exe"),
                p("C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"),
            ]
        );
        assert_eq!(
            channel_paths("chrome-canary", &windows).unwrap()[0],
            p("C:/Users/u/AppData/Local/Google/Chrome SxS/Application/chrome.exe")
        );
        // Linux has no canary; unknown names are errors, like Playwright.
        assert!(channel_paths("chrome-canary", &linux).is_err());
        let err = channel_paths("opera", &linux).unwrap_err();
        assert!(err.contains("opera"), "{err}");
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
        let system: Vec<_> = got[10..].to_vec();
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
