//! Launch configuration: Playwright's default Chromium argument preset.

use std::path::{Path, PathBuf};

/// Which kind of Chromium binary is being launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserKind {
    /// `chrome-headless-shell` (Playwright's `chromium_headless_shell-*`): old headless mode.
    HeadlessShell,
    /// Full Chrome / Chromium / Chrome for Testing: `--headless=new`.
    Chrome,
}

impl BrowserKind {
    /// Infers the kind from the executable path.
    pub fn from_executable(_path: &Path) -> Self {
        unimplemented!()
    }
}

/// Everything needed to build the browser command line.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpec {
    pub kind: BrowserKind,
    pub headless: bool,
    /// Extra arguments appended after the preset.
    pub args: Vec<String>,
    /// Skip the preset entirely (Playwright's `ignoreDefaultArgs: true`).
    pub ignore_default_args: bool,
    /// The (unique, temporary) profile directory.
    pub user_data_dir: PathBuf,
}

/// Playwright's default Chromium arguments, before the headless-related ones.
pub fn default_args() -> Vec<String> {
    unimplemented!()
}

/// The full command line (without the program) for a launch.
pub fn command_line(_spec: &LaunchSpec) -> Vec<String> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from a real Playwright 1.63 launch (device-mockup contract).
    const PRESET: &str = "--font-render-hinting=none --disable-lcd-text --force-color-profile=srgb \
--disable-blink-features=XSLT,EnableXSLTForCAPAlerts --enable-features=SendGPUChannelEarly,CDPScreenshotNewSurface \
--js-flags=--no-enable-lazy-source-positions --disable-field-trial-config --disable-background-networking \
--disable-background-timer-throttling --disable-backgrounding-occluded-windows --disable-back-forward-cache \
--disable-breakpad --disable-client-side-phishing-detection --disable-component-extensions-with-background-pages \
--disable-component-update --no-default-browser-check --disable-default-apps --disable-extensions \
--disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,DestroyProfileOnBrowserClose,DialMediaRouteProvider,GlobalMediaControls,HttpsUpgrades,LensOverlay,MediaRouter,PaintHolding,ThirdPartyStoragePartitioning,BlockOriginHeaderModificationOnRedirect,Translate,AutoDeElevate,OptimizationHints \
--allow-pre-commit-input --disable-hang-monitor --disable-ipc-flooding-protection --disable-popup-blocking \
--disable-prompt-on-repost --disable-renderer-backgrounding --metrics-recording-only --no-first-run \
--password-store=basic --use-mock-keychain --no-service-autorun --export-tagged-pdf \
--disable-search-engine-choice-screen --unsafely-disable-devtools-self-xss-warnings --disable-sync \
--enable-unsafe-swiftshader";

    const HEADLESS_TAIL: &str = "--hide-scrollbars --mute-audio \
--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4";

    fn split(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    fn spec(kind: BrowserKind, headless: bool) -> LaunchSpec {
        LaunchSpec {
            kind,
            headless,
            args: vec![],
            ignore_default_args: false,
            user_data_dir: PathBuf::from("/tmp/profile-1"),
        }
    }

    #[test]
    fn preset_matches_playwright_exactly() {
        assert_eq!(default_args(), split(PRESET));
    }

    #[test]
    fn headless_shell_uses_old_headless_mode() {
        let mut want = split(PRESET);
        want.push("--headless".into());
        want.extend(split(HEADLESS_TAIL));
        want.extend(split(
            "--no-sandbox --user-data-dir=/tmp/profile-1 --remote-debugging-pipe --no-startup-window",
        ));
        assert_eq!(command_line(&spec(BrowserKind::HeadlessShell, true)), want);
    }

    #[test]
    fn full_chrome_uses_new_headless_mode() {
        let args = command_line(&spec(BrowserKind::Chrome, true));
        assert!(args.contains(&"--headless=new".to_owned()));
        assert!(!args.contains(&"--headless".to_owned()));
    }

    #[test]
    fn headful_has_no_headless_args() {
        let args = command_line(&spec(BrowserKind::Chrome, false));
        for a in ["--headless", "--headless=new", "--hide-scrollbars", "--mute-audio"] {
            assert!(!args.contains(&a.to_owned()), "{a} present");
        }
        assert!(args.contains(&"--no-sandbox".to_owned()));
    }

    #[test]
    fn user_args_come_after_preset_and_before_profile() {
        let mut s = spec(BrowserKind::HeadlessShell, true);
        s.args = vec!["--lang=de".into()];
        let args = command_line(&s);
        let user = args.iter().position(|a| a == "--lang=de").unwrap();
        let sandbox = args.iter().position(|a| a == "--no-sandbox").unwrap();
        let profile = args.iter().position(|a| a.starts_with("--user-data-dir=")).unwrap();
        assert!(sandbox < user && user < profile);
    }

    #[test]
    fn ignore_default_args_keeps_only_essentials() {
        let mut s = spec(BrowserKind::HeadlessShell, true);
        s.ignore_default_args = true;
        s.args = vec!["--foo".into()];
        assert_eq!(
            command_line(&s),
            split("--foo --user-data-dir=/tmp/profile-1 --remote-debugging-pipe --no-startup-window")
        );
    }

    #[test]
    fn kind_is_inferred_from_the_executable_name() {
        use BrowserKind::*;
        for (p, k) in [
            ("/opt/pw-browsers/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell", HeadlessShell),
            ("/x/chrome-headless-shell-mac-arm64/chrome-headless-shell", HeadlessShell),
            ("C:\\x\\chrome-headless-shell.exe", HeadlessShell),
            ("/x/headless_shell", HeadlessShell),
            ("/opt/pw-browsers/chromium-1243/chrome-linux64/chrome", Chrome),
            ("/usr/bin/google-chrome-stable", Chrome),
            ("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", Chrome),
        ] {
            assert_eq!(BrowserKind::from_executable(Path::new(p)), k, "{p}");
        }
    }
}
