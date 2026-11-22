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
    pub fn from_executable(path: &Path) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if name.contains("headless-shell") || name.contains("headless_shell") {
            BrowserKind::HeadlessShell
        } else {
            BrowserKind::Chrome
        }
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
    /// `LaunchOptions::proxy`.
    pub proxy: Option<Proxy>,
}

/// Playwright's `proxy` option (launch or context).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Proxy {
    /// `http://myproxy.com:3128` or `socks5://myproxy.com:3128`.
    pub server: String,
    /// Comma-separated domains that skip the proxy (`.com, chromium.org`).
    pub bypass: Option<String>,
    /// Credentials for the proxy's own challenges (407) only (Playwright 1.65).
    pub username: Option<String>,
    pub password: Option<String>,
}

/// Playwright's `shouldProxyLoopback`: loopback hosts go through the proxy
/// unless the bypass list names one.
pub fn should_proxy_loopback(bypass: Option<&str>) -> bool {
    const LOOPBACK: [&str; 7] = [
        "localhost",
        "127.0.0.1",
        "::1",
        "[::]",
        "[::1]",
        "<loopback>",
        "<-loopback>",
    ];
    !bypass
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .any(|h| LOOPBACK.contains(&h))
}

/// The `--proxy-server` / `--proxy-bypass-list` switches of a launch.
pub fn proxy_args(proxy: &Proxy) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(u) = url::Url::parse(&proxy.server)
        && u.scheme() == "socks5"
    {
        out.push(format!(
            "--host-resolver-rules=\"MAP * ~NOTFOUND , EXCLUDE {}\"",
            u.host_str().unwrap_or_default()
        ));
    }
    out.push(format!("--proxy-server={}", proxy.server));
    let mut rules: Vec<String> = proxy
        .bypass
        .as_deref()
        .map(|b| {
            b.split(',')
                .map(str::trim)
                .map(|t| {
                    if t.starts_with('.') {
                        format!("*{t}")
                    } else {
                        t.to_owned()
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if should_proxy_loopback(proxy.bypass.as_deref()) {
        rules.push("<-loopback>".into());
    }
    if !rules.is_empty() {
        out.push(format!("--proxy-bypass-list={}", rules.join(";")));
    }
    out
}

/// The `proxyBypassList` of `Target.createBrowserContext`.
pub fn context_proxy_bypass_list(proxy: &Proxy) -> Option<String> {
    if should_proxy_loopback(proxy.bypass.as_deref()) {
        Some(match &proxy.bypass {
            Some(b) => format!("<-loopback>,{b}"),
            None => "<-loopback>".to_owned(),
        })
    } else {
        proxy.bypass.clone()
    }
}

/// Playwright's default Chromium arguments, before the headless-related ones.
pub fn default_args() -> Vec<String> {
    DEFAULT_ARGS.iter().map(|a| (*a).to_owned()).collect()
}

/// The full command line (without the program) for a launch.
pub fn command_line(spec: &LaunchSpec) -> Vec<String> {
    let mut args = Vec::new();
    if !spec.ignore_default_args {
        args.extend(default_args());
        if spec.headless {
            args.push(match spec.kind {
                BrowserKind::HeadlessShell => "--headless".to_owned(),
                BrowserKind::Chrome => "--headless=new".to_owned(),
            });
            args.extend(HEADLESS_ARGS.iter().map(|a| (*a).to_owned()));
        }
        args.push("--no-sandbox".to_owned());
        if let Some(p) = &spec.proxy {
            args.extend(proxy_args(p));
        }
    }
    args.extend(spec.args.iter().cloned());
    args.push(format!("--user-data-dir={}", spec.user_data_dir.display()));
    args.push("--remote-debugging-pipe".to_owned());
    args.push("--no-startup-window".to_owned());
    args
}

/// The command line for a persistent context (`launchPersistentContext`):
/// like [`command_line`], but opening `about:blank` instead of no window.
pub fn persistent_command_line(spec: &LaunchSpec) -> Vec<String> {
    let mut args = command_line(spec);
    if let Some(last) = args.last_mut() {
        *last = "about:blank".to_owned();
    }
    args
}

/// Playwright 1.65.1's default Chromium switches (device-mockup contract; do
/// not edit without telling device-mockup).
const DEFAULT_ARGS: &[&str] = &[
    "--font-render-hinting=none",
    "--disable-lcd-text",
    "--force-color-profile=srgb",
    "--disable-blink-features=XSLT,EnableXSLTForCAPAlerts",
    "--enable-features=SendGPUChannelEarly,CDPScreenshotNewSurface,SkipListAccountsWithoutGaiaCookies",
    "--js-flags=--no-enable-lazy-source-positions",
    "--disable-field-trial-config",
    "--disable-background-networking",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-back-forward-cache",
    "--disable-breakpad",
    "--disable-component-extensions-with-background-pages",
    "--disable-component-update",
    "--no-default-browser-check",
    "--disable-default-apps",
    "--disable-extensions",
    "--disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,DestroyProfileOnBrowserClose,\
HttpsUpgrades,HttpsFirstBalancedModeAutoEnable,LensOverlay,MediaRouter,PaintHolding,\
ThirdPartyStoragePartitioning,AvoidCorsURLLoaderRestartOnRedirect,TcpPortRandomizationWin,\
OptimizationHints,NetworkTimeServiceQuerying,AimEnabled,PreconnectToSearch",
    "--allow-pre-commit-input",
    "--disable-hang-monitor",
    "--disable-ipc-flooding-protection",
    "--disable-popup-blocking",
    "--disable-prompt-on-repost",
    "--disable-renderer-backgrounding",
    "--metrics-recording-only",
    "--no-first-run",
    "--password-store=basic",
    "--use-mock-keychain",
    "--no-service-autorun",
    "--disable-search-engine-choice-screen",
    "--unsafely-disable-devtools-self-xss-warnings",
    "--enable-unsafe-swiftshader",
    "--disable-stack-profiler",
    "--disable-sync",
    "--do-not-de-elevate",
];

/// Added after `--headless` in headless mode.
const HEADLESS_ARGS: &[&str] = &[
    "--hide-scrollbars",
    "--mute-audio",
    "--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The Playwright 1.65.1 preset (device-mockup contract, agreed with the
    /// device-mockup session): `PRESET_1_63` with Playwright's 1.63 -> 1.65
    /// switch changes, as read from `/proc/<pid>/cmdline` of a real 1.65.1
    /// launch of the same browser (Edge-only switches and repeats left out,
    /// as in 1.63).
    const PRESET: &str = "--font-render-hinting=none --disable-lcd-text --force-color-profile=srgb \
--disable-blink-features=XSLT,EnableXSLTForCAPAlerts \
--enable-features=SendGPUChannelEarly,CDPScreenshotNewSurface,SkipListAccountsWithoutGaiaCookies \
--js-flags=--no-enable-lazy-source-positions --disable-field-trial-config --disable-background-networking \
--disable-background-timer-throttling --disable-backgrounding-occluded-windows --disable-back-forward-cache \
--disable-breakpad --disable-component-extensions-with-background-pages \
--disable-component-update --no-default-browser-check --disable-default-apps --disable-extensions \
--disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,DestroyProfileOnBrowserClose,HttpsUpgrades,HttpsFirstBalancedModeAutoEnable,LensOverlay,MediaRouter,PaintHolding,ThirdPartyStoragePartitioning,AvoidCorsURLLoaderRestartOnRedirect,TcpPortRandomizationWin,OptimizationHints,NetworkTimeServiceQuerying,AimEnabled,PreconnectToSearch \
--allow-pre-commit-input --disable-hang-monitor --disable-ipc-flooding-protection --disable-popup-blocking \
--disable-prompt-on-repost --disable-renderer-backgrounding --metrics-recording-only --no-first-run \
--password-store=basic --use-mock-keychain --no-service-autorun \
--disable-search-engine-choice-screen --unsafely-disable-devtools-self-xss-warnings \
--enable-unsafe-swiftshader --disable-stack-profiler --disable-sync --do-not-de-elevate";

    /// The previous preset, captured from a real Playwright 1.63 launch; kept
    /// so device-mockup can trace any pixel difference to a switch.
    #[allow(dead_code)]
    const PRESET_1_63: &str = "--font-render-hinting=none --disable-lcd-text --force-color-profile=srgb \
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
            proxy: None,
        }
    }

    #[test]
    fn proxy_switches_like_playwright() {
        let p = |server: &str, bypass: Option<&str>| Proxy {
            server: server.into(),
            bypass: bypass.map(str::to_owned),
            ..Default::default()
        };
        assert_eq!(
            proxy_args(&p("http://p.test:8080", None)),
            [
                "--proxy-server=http://p.test:8080",
                "--proxy-bypass-list=<-loopback>"
            ]
        );
        assert_eq!(
            proxy_args(&p("http://p.test:8080", Some(".example.com, localhost"))),
            [
                "--proxy-server=http://p.test:8080",
                "--proxy-bypass-list=*.example.com;localhost"
            ]
        );
        assert_eq!(
            proxy_args(&p("socks5://s.test:1080", Some("a.test"))),
            [
                "--host-resolver-rules=\"MAP * ~NOTFOUND , EXCLUDE s.test\"",
                "--proxy-server=socks5://s.test:1080",
                "--proxy-bypass-list=a.test;<-loopback>"
            ]
        );
        assert!(!should_proxy_loopback(Some("x.test, 127.0.0.1")));
        assert!(should_proxy_loopback(None));
        assert_eq!(
            context_proxy_bypass_list(&p("http://p.test", None)).as_deref(),
            Some("<-loopback>")
        );
        assert_eq!(
            context_proxy_bypass_list(&p("http://p.test", Some("a.test"))).as_deref(),
            Some("<-loopback>,a.test")
        );
        assert_eq!(
            context_proxy_bypass_list(&p("http://p.test", Some("localhost"))).as_deref(),
            Some("localhost")
        );
        // Placed after --no-sandbox, before the user's arguments.
        let mut s = spec(BrowserKind::Chrome, false);
        s.proxy = Some(p("http://p.test:8080", None));
        s.args = vec!["--lang=de".into()];
        let args = command_line(&s);
        let at = |a: &str| args.iter().position(|x| x == a).unwrap();
        assert_eq!(
            at("--proxy-server=http://p.test:8080"),
            at("--no-sandbox") + 1
        );
        assert_eq!(at("--lang=de"), at("--proxy-bypass-list=<-loopback>") + 1);
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
        for a in [
            "--headless",
            "--headless=new",
            "--hide-scrollbars",
            "--mute-audio",
        ] {
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
        let profile = args
            .iter()
            .position(|a| a.starts_with("--user-data-dir="))
            .unwrap();
        assert!(sandbox < user && user < profile);
    }

    #[test]
    fn ignore_default_args_keeps_only_essentials() {
        let mut s = spec(BrowserKind::HeadlessShell, true);
        s.ignore_default_args = true;
        s.args = vec!["--foo".into()];
        assert_eq!(
            command_line(&s),
            split(
                "--foo --user-data-dir=/tmp/profile-1 --remote-debugging-pipe --no-startup-window"
            )
        );
    }

    #[test]
    fn kind_is_inferred_from_the_executable_name() {
        use BrowserKind::*;
        for (p, k) in [
            (
                "/opt/pw-browsers/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell",
                HeadlessShell,
            ),
            (
                "/x/chrome-headless-shell-mac-arm64/chrome-headless-shell",
                HeadlessShell,
            ),
            ("C:\\x\\chrome-headless-shell.exe", HeadlessShell),
            ("/x/headless_shell", HeadlessShell),
            (
                "/opt/pw-browsers/chromium-1243/chrome-linux64/chrome",
                Chrome,
            ),
            ("/usr/bin/google-chrome-stable", Chrome),
            (
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                Chrome,
            ),
        ] {
            assert_eq!(BrowserKind::from_executable(Path::new(p)), k, "{p}");
        }
    }
}

#[cfg(test)]
mod persistent_tests {
    use super::*;

    #[test]
    fn persistent_launches_open_about_blank_like_playwright() {
        let spec = LaunchSpec {
            kind: BrowserKind::HeadlessShell,
            headless: true,
            args: Vec::new(),
            ignore_default_args: false,
            user_data_dir: "/tmp/profile-1".into(),
            proxy: None,
        };
        let args = persistent_command_line(&spec);
        assert_eq!(args.last().map(String::as_str), Some("about:blank"));
        assert!(!args.iter().any(|a| a == "--no-startup-window"));
    }
}
