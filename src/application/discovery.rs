//! Use case: find the browser executable to launch.

use std::path::{Path, PathBuf};

use crate::domain::discovery::{
    DiscoveryEnv, Os, Probe, Source, channel_paths, executables_within, probes, rank_entries,
};
use crate::domain::error::{Error, Result};
use crate::domain::launch::BrowserKind;
use crate::ports::{Environment, FileSystem};

/// A browser found by discovery.
#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    pub executable: PathBuf,
    pub kind: BrowserKind,
    pub source: Source,
}

/// Builds the discovery snapshot from the environment port.
pub fn discovery_env(env: &dyn Environment, explicit: Option<&Path>) -> DiscoveryEnv {
    let os = env.os();
    let var = |name: &str| env.var(name).filter(|v| !v.is_empty());
    let separator = if os == Os::Windows { ';' } else { ':' };
    DiscoveryEnv {
        os,
        explicit: explicit.map(Path::to_path_buf),
        mockup_chromium: var("MOCKUP_CHROMIUM").map(PathBuf::from),
        chrome: var("CHROME").map(PathBuf::from),
        playwright_browsers_path: var("PLAYWRIGHT_BROWSERS_PATH").map(PathBuf::from),
        home: env.home_dir(),
        path: var("PATH")
            .map(|p| {
                p.split(separator)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default(),
        program_files: ["ProgramFiles", "ProgramFiles(x86)"]
            .into_iter()
            .filter_map(|k| var(k).map(PathBuf::from))
            .collect(),
        local_app_data: var("LOCALAPPDATA").map(PathBuf::from),
    }
}

/// Finds the browser: explicit path, env vars, Playwright installs, Chrome for
/// Testing, then system Chrome.
pub fn discover(
    fs: &dyn FileSystem,
    env: &dyn Environment,
    explicit: Option<&Path>,
) -> Result<Discovered> {
    let snapshot = discovery_env(env, explicit);
    let found = |executable: PathBuf, source| Discovered {
        kind: BrowserKind::from_executable(&executable),
        executable,
        source,
    };
    let mut tried = Vec::new();
    for probe in probes(&snapshot) {
        match probe {
            Probe::Required { path, source } => {
                if fs.is_file(&path) {
                    return Ok(found(path, source));
                }
                return Err(Error::ExecutableMissing {
                    path,
                    origin: source,
                });
            }
            Probe::Candidate { path, source } => {
                if fs.is_file(&path) {
                    return Ok(found(path, source));
                }
                tried.push(path);
            }
            Probe::Root {
                root,
                layout,
                source,
            } => {
                if fs.is_dir(&root) {
                    for entry in rank_entries(layout, &fs.list_dir(&root)) {
                        for rel in executables_within(snapshot.os) {
                            let candidate = root.join(&entry).join(rel);
                            if fs.is_file(&candidate) {
                                return Ok(found(candidate, source));
                            }
                        }
                    }
                }
                tried.push(root);
            }
        }
    }
    Err(Error::BrowserNotFound { tried })
}

/// Finds the installed browser of a Playwright `channel` (`msedge`,
/// `chrome-beta`, ...); the default discovery is not consulted.
pub fn discover_channel(
    fs: &dyn FileSystem,
    env: &dyn Environment,
    channel: &str,
) -> Result<Discovered> {
    let snapshot = discovery_env(env, None);
    let paths = channel_paths(channel, &snapshot).map_err(Error::InvalidArgument)?;
    match paths.iter().find(|p| fs.is_file(p)) {
        Some(path) => Ok(Discovered {
            kind: BrowserKind::from_executable(path),
            executable: path.clone(),
            source: Source::System,
        }),
        None => Err(Error::Launch(format!(
            "Chromium distribution '{channel}' is not found at {}",
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::memory::{MapEnvironment, MemoryFileSystem};

    const PW_SHELL: &str = "/opt/pw-browsers/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell";
    const PW_CHROME: &str = "/opt/pw-browsers/chromium-1243/chrome-linux64/chrome";

    fn found(fs: &MemoryFileSystem, env: &MapEnvironment, explicit: Option<&str>) -> Discovered {
        discover(fs, env, explicit.map(Path::new)).expect("discovered")
    }

    #[test]
    fn explicit_path_wins() {
        let fs = MemoryFileSystem::with_files(["/my/chrome", PW_SHELL]);
        let env = MapEnvironment::linux("/home/u").set("MOCKUP_CHROMIUM", PW_SHELL);
        let d = found(&fs, &env, Some("/my/chrome"));
        assert_eq!(d.executable, PathBuf::from("/my/chrome"));
        assert_eq!(d.source, Source::Explicit);
        assert_eq!(d.kind, BrowserKind::Chrome);
    }

    #[test]
    fn missing_explicit_path_is_an_error_not_a_fallback() {
        let fs = MemoryFileSystem::with_files([PW_SHELL]);
        let env = MapEnvironment::linux("/home/u");
        let err = discover(&fs, &env, Some(Path::new("/nope"))).unwrap_err();
        assert!(
            matches!(
                err,
                Error::ExecutableMissing {
                    origin: Source::Explicit,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn mockup_chromium_then_chrome_env() {
        let fs = MemoryFileSystem::with_files(["/a/chrome", "/b/chrome", PW_SHELL]);
        let env = MapEnvironment::linux("/home/u")
            .set("MOCKUP_CHROMIUM", "/a/chrome")
            .set("CHROME", "/b/chrome");
        assert_eq!(found(&fs, &env, None).source, Source::MockupChromiumEnv);
        let env = MapEnvironment::linux("/home/u").set("CHROME", "/b/chrome");
        let d = found(&fs, &env, None);
        assert_eq!(
            (d.executable, d.source),
            (PathBuf::from("/b/chrome"), Source::ChromeEnv)
        );
    }

    #[test]
    fn playwright_browsers_path_before_opt() {
        let fs = MemoryFileSystem::with_files([
            PW_SHELL,
            "/pwb/chromium_headless_shell-1000/chrome-headless-shell-linux64/chrome-headless-shell",
        ]);
        let env = MapEnvironment::linux("/home/u").set("PLAYWRIGHT_BROWSERS_PATH", "/pwb");
        let d = found(&fs, &env, None);
        assert_eq!(d.source, Source::PlaywrightBrowsersPath);
        assert_eq!(d.kind, BrowserKind::HeadlessShell);
    }

    #[test]
    fn opt_pw_browsers_prefers_headless_shell() {
        let fs = MemoryFileSystem::with_files([PW_CHROME, PW_SHELL]);
        let d = found(&fs, &MapEnvironment::linux("/home/u"), None);
        assert_eq!(d.executable, PathBuf::from(PW_SHELL));
        assert_eq!(d.source, Source::OptPwBrowsers);
    }

    #[test]
    fn newest_revision_with_an_executable_wins() {
        let fs = MemoryFileSystem::with_files([
            "/home/u/.cache/ms-playwright/chromium-1100/chrome-linux/chrome",
            "/home/u/.cache/ms-playwright/chromium-1200/chrome-linux64/chrome",
        ]);
        fs.add_file("/home/u/.cache/ms-playwright/chromium-1300/INSTALLATION_INCOMPLETE");
        let d = found(&fs, &MapEnvironment::linux("/home/u"), None);
        assert_eq!(
            d.executable,
            PathBuf::from("/home/u/.cache/ms-playwright/chromium-1200/chrome-linux64/chrome")
        );
        assert_eq!(d.source, Source::PlaywrightCache);
    }

    #[test]
    fn chrome_for_testing_then_system_chrome() {
        let cft = "/home/u/.cache/chromiumoxide/linux-140.0.7339.5/chrome-linux64/chrome";
        let fs = MemoryFileSystem::with_files([cft, "/usr/bin/google-chrome-stable"]);
        let env = MapEnvironment::linux("/home/u").set("PATH", "/usr/local/bin:/usr/bin");
        assert_eq!(found(&fs, &env, None).source, Source::ChromeForTesting);
        let fs = MemoryFileSystem::with_files(["/usr/bin/google-chrome-stable"]);
        let d = found(&fs, &env, None);
        assert_eq!(
            (d.executable, d.source),
            (
                PathBuf::from("/usr/bin/google-chrome-stable"),
                Source::System
            )
        );
    }

    #[test]
    fn nothing_found_lists_where_it_looked() {
        let fs = MemoryFileSystem::default();
        let err = discover(&fs, &MapEnvironment::linux("/home/u"), None).unwrap_err();
        match err {
            Error::BrowserNotFound { tried } => {
                assert!(tried.contains(&PathBuf::from("/opt/pw-browsers")))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn env_snapshot_reads_variables() {
        let env = MapEnvironment::linux("/home/u")
            .set("PATH", "/a:/b")
            .set("PLAYWRIGHT_BROWSERS_PATH", "/pwb")
            .set("MOCKUP_CHROMIUM", "/m")
            .set("CHROME", "");
        let snap = discovery_env(&env, Some(Path::new("/x")));
        assert_eq!(snap.explicit, Some(PathBuf::from("/x")));
        assert_eq!(snap.mockup_chromium, Some(PathBuf::from("/m")));
        assert_eq!(snap.chrome, None);
        assert_eq!(snap.path, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(snap.home, Some(PathBuf::from("/home/u")));
    }
}
