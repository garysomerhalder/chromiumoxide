//! The browser extension (`extension/`, WXT): the library drives a browser
//! that was not started for automation, through the extension.

use std::path::{Path, PathBuf};
use std::process::Command;

use chromiumoxide::{Chromium, ContextOptions, ExtensionRelayOptions, LaunchOptions};

use crate::server::TestServer;

/// Builds the extension for `browser` (`chrome`, `firefox`) once and
/// returns its output folder; `None` (skip) without npm, unless
/// `CHROMIUMOXIDE_REQUIRE_EXTENSION` is set.
pub(crate) fn built(browser: &str) -> Option<PathBuf> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("extension");
    let out = dir.join(".output").join(match browser {
        "firefox" => "firefox-mv2",
        "safari" => "safari-mv3",
        _ => "chrome-mv3",
    });
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let run = |args: &[&str]| {
        Command::new(npm)
            .args(args)
            .current_dir(&dir)
            .status()
            .is_ok_and(|s| s.success())
    };
    let ok = (dir.join("node_modules").exists() || run(&["ci"]))
        && run(&["run", &format!("build:{browser}")]);
    if !ok || !out.join("manifest.json").exists() {
        assert!(
            std::env::var_os("CHROMIUMOXIDE_REQUIRE_EXTENSION").is_none(),
            "the extension did not build for {browser}"
        );
        eprintln!("npm missing or the extension did not build; skipping");
        return None;
    }
    Some(out)
}

/// Playwright's full Chromium (its headless shell cannot load extensions).
pub(crate) fn full_chromium() -> Option<PathBuf> {
    let root = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("LOCALAPPDATA"))?;
            let home = PathBuf::from(home);
            [
                PathBuf::from("/opt/pw-browsers"),
                home.join(".cache/ms-playwright"),
                home.join("ms-playwright"),
                home.join("Library/Caches/ms-playwright"),
            ]
            .into_iter()
            .find(|p| p.exists())
        })?;
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("chromium-"))
        })
        .collect();
    dirs.sort();
    let dir = dirs.pop()?;
    [
        "chrome-linux64/chrome",
        "chrome-linux/chrome",
        "chrome-win64/chrome.exe",
        "chrome-win/chrome.exe",
        "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
    ]
    .iter()
    .map(|p| dir.join(p))
    .find(|p| p.exists())
}

#[tokio::test]
async fn extension_drives_an_everyday_chromium() {
    let (Some(ext), Some(chromium)) = (built("chrome"), full_chromium()) else {
        return;
    };
    let server = TestServer::start().await;
    server.route(
        "/hello",
        "text/html",
        "<title>Hello</title><button onclick=\"this.textContent='clicked'\">Go</button>",
        0,
    );

    // Gary's browser: his own profile, with the extension loaded and a tab
    // already open. Launched here only to have one; the library reaches it
    // through the extension alone.
    let ext_arg = ext.display().to_string();
    let profile = std::env::temp_dir().join(format!("cox-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&profile);
    let everyday = Chromium::launch_persistent_context(
        &profile,
        LaunchOptions {
            executable_path: Some(chromium),
            args: vec![
                format!("--load-extension={ext_arg}"),
                format!("--disable-extensions-except={ext_arg}"),
            ],
            ignored_default_args: vec!["--disable-extensions".into()],
            ..Default::default()
        },
        ContextOptions::default(),
    )
    .await
    .unwrap();
    let mine = everyday.new_page().await.unwrap();
    mine.goto(&server.url("/hello")).await.unwrap();

    let relay = Chromium::extension_relay(ExtensionRelayOptions::default())
        .await
        .unwrap();
    // What `Chromium::connect_extension` does with the default browser:
    // opening the relay's page tells the extension where to connect.
    let connect_tab = everyday.new_page().await.unwrap();
    connect_tab.goto(&relay.connect_url()).await.unwrap();
    let browser = relay.wait().await.unwrap();

    // The existing tabs, in the browser's own profile.
    let ctx = browser
        .contexts()
        .into_iter()
        .next()
        .expect("the browser's own context");
    let hello = ctx
        .pages()
        .into_iter()
        .find(|p| p.url().ends_with("/hello"))
        .expect("the tab that was already open");
    assert_eq!(hello.title().await.unwrap(), "Hello");
    hello.locator("button").click().await.unwrap();
    assert_eq!(
        hello.locator("button").inner_text().await.unwrap(),
        "clicked"
    );

    // A new tab through the extension.
    let page = ctx.new_page().await.unwrap();
    page.goto(&server.url("/hello")).await.unwrap();
    assert_eq!(page.evaluate::<i64>("6 * 7").await.unwrap(), 42);
    let png = page.screenshot().await.unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    page.close().await.unwrap();

    // Disconnecting leaves the browser and its tabs alone.
    browser.close().await.unwrap();
    assert_eq!(
        mine.locator("button").inner_text().await.unwrap(),
        "clicked"
    );
    everyday.close().await.unwrap();
    let _ = std::fs::remove_dir_all(&profile);
}

#[tokio::test]
async fn extension_drives_firefox_tabs() {
    let (Some(ext), Some(options)) = (built("firefox"), crate::firefox::options()) else {
        return;
    };
    let server = TestServer::start().await;
    server.route(
        "/form",
        "text/html",
        "<title>Form</title><input><button onclick=\"p.textContent='Hello '+document.querySelector('input').value\">Go</button><p id=p></p>",
        0,
    );
    // Gary's Firefox, with the extension installed and a tab open.
    let firefox = chromiumoxide::Firefox::launch(options).await.unwrap();
    firefox.install_extension(&ext).await.unwrap();
    let mine = firefox.new_page().await.unwrap();
    mine.goto(&server.url("/form")).await.unwrap();

    let relay = Chromium::extension_relay(ExtensionRelayOptions::default())
        .await
        .unwrap();
    let connect_tab = firefox.new_page().await.unwrap();
    connect_tab.goto(&relay.connect_url()).await.unwrap();
    let browser = relay.wait_pages().await.unwrap();

    let tab = browser
        .tabs()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.url().ends_with("/form"))
        .expect("the tab that was already open");
    assert_eq!(tab.title(), "Form");
    tab.fill("input", "Gary").await.unwrap();
    tab.click("button").await.unwrap();
    assert_eq!(
        tab.evaluate::<String>("document.querySelector('p').textContent")
            .await
            .unwrap(),
        "Hello Gary"
    );
    assert!(tab.click("#missing").await.is_err());

    let new = browser.new_tab(&server.url("/form")).await.unwrap();
    new.goto(&server.url("/form?again")).await.unwrap();
    assert!(new.url().ends_with("/form?again"));
    assert_eq!(new.evaluate::<i64>("6 * 7").await.unwrap(), 42);
    let png = new.screenshot().await.unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    new.close().await.unwrap();

    // Disconnecting leaves Firefox and its tabs alone.
    browser.close().await;
    assert_eq!(mine.title().await.unwrap(), "Form");
    firefox.close().await.unwrap();
}
