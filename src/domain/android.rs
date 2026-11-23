//! Playwright's `_android`, the pure parts: Chrome's command line and what
//! adb prints (device list, DevTools sockets, process list).

/// Playwright 1.65.1's `chromiumSwitches({ android: true })`: the shared
/// Chromium switches, without `--disable-sync`.
const ANDROID_SWITCHES: &[&str] = &[
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
    "--disable-dev-shm-usage",
    "--disable-edgeupdater",
    "--disable-extensions",
    "--disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,DestroyProfileOnBrowserClose,HttpsUpgrades,HttpsFirstBalancedModeAutoEnable,LensOverlay,MediaRouter,PaintHolding,ThirdPartyStoragePartitioning,AvoidCorsURLLoaderRestartOnRedirect,TcpPortRandomizationWin,OptimizationHints,NetworkTimeServiceQuerying,AimEnabled,PreconnectToSearch,msForceBrowserSignIn,msEdgeUpdateLaunchServicesPreferredVersion",
    "--enable-features=CDPScreenshotNewSurface,SkipListAccountsWithoutGaiaCookies",
    "--allow-pre-commit-input",
    "--disable-hang-monitor",
    "--disable-ipc-flooding-protection",
    "--disable-popup-blocking",
    "--disable-prompt-on-repost",
    "--disable-renderer-backgrounding",
    "--disable-updater-scheduler",
    "--force-color-profile=srgb",
    "--metrics-recording-only",
    "--no-first-run",
    "--password-store=basic",
    "--use-mock-keychain",
    "--no-service-autorun",
    "--disable-search-engine-choice-screen",
    "--unsafely-disable-devtools-self-xss-warnings",
    "--enable-unsafe-swiftshader",
    "--edge-skip-compat-layer-relaunch",
    "--disable-stack-profiler",
    "--disable-infobars",
    "--do-not-de-elevate",
];

/// The `chrome-command-line` file Playwright writes before starting Chrome
/// (`_defaultArgs`): a dummy program name, Playwright's switches, then `args`.
pub fn chrome_command_line(args: &[String], socket_name: &str) -> String {
    let mut line = vec![
        "_".to_owned(),
        "--disable-fre".to_owned(),
        "--no-default-browser-check".to_owned(),
        format!("--remote-debugging-socket-name={socket_name}"),
    ];
    line.extend(ANDROID_SWITCHES.iter().map(|s| (*s).to_owned()));
    line.extend(args.iter().cloned());
    line.join(" ")
}

/// `host:devices`: `(serial, state)` per line.
pub fn parse_devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
        })
        .collect()
}

/// `cat /proc/net/unix | grep webview_devtools_remote`: `(socket name, pid)`;
/// the pid is `-1` when the name has none.
pub fn parse_webview_sockets(text: &str) -> Vec<(String, i64)> {
    text.lines()
        .filter_map(|l| {
            let (_, name) = l.split_once('@')?;
            let name = name.trim();
            let at = name.find("webview_devtools_remote")?;
            let rest = &name[at + "webview_devtools_remote".len()..];
            let digits: String = rest
                .trim_start_matches('_')
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            let pid = digits.parse().unwrap_or(-1);
            Some((name.to_owned(), pid))
        })
        .collect()
}

/// `ps -A | grep <pid>`: the package (last column) of the process `pid`.
pub fn package_of(ps: &str, pid: i64) -> String {
    ps.lines()
        .find(|l| {
            l.split_whitespace()
                .nth(1)
                .and_then(|p| p.parse::<i64>().ok())
                == Some(pid)
        })
        .and_then(|l| l.split_whitespace().last())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_like_playwright() {
        let line = chrome_command_line(&["--lang=de".into()], "sock");
        assert!(line.starts_with(
            "_ --disable-fre --no-default-browser-check --remote-debugging-socket-name=sock --disable-field-trial-config "
        ));
        assert!(line.ends_with(" --disable-infobars --do-not-de-elevate --lang=de"));
        assert!(!line.contains("--disable-sync"));
    }

    #[test]
    fn parses_adb_output() {
        assert_eq!(
            parse_devices("emulator-5554\tdevice\nR58M\tunauthorized\n"),
            [
                ("emulator-5554".into(), "device".into()),
                ("R58M".into(), "unauthorized".into())
            ]
        );
        let unix = "0000: 00000002 0 10000 0001 01 1 @webview_devtools_remote_4242\n\
                    0000: 00000002 0 10000 0001 01 2 @chrome_devtools_remote\n\
                    0000: 00000002 0 10000 0001 01 3 @webview_devtools_remote\n";
        assert_eq!(
            parse_webview_sockets(unix),
            [
                ("webview_devtools_remote_4242".into(), 4242),
                ("webview_devtools_remote".into(), -1)
            ]
        );
        let ps = "u0_a1  42420 1 0 S other\nu0_a123  4242 100 0 S com.example.app\n";
        assert_eq!(package_of(ps, 4242), "com.example.app");
        assert_eq!(package_of(ps, 7), "");
    }
}
