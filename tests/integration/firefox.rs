//! The Firefox engine (`chromiumoxide::firefox`, over WebDriver BiDi)
//! against Playwright's Firefox build.

use chromiumoxide::Firefox;
use chromiumoxide::firefox::FirefoxLaunchOptions;

/// Playwright's newest Firefox (`firefox-<revision>/firefox/firefox`). With
/// `CHROMIUMOXIDE_REQUIRE_FIREFOX` set, a missing Firefox fails the test
/// instead of skipping it.
pub fn firefox_executable() -> Option<std::path::PathBuf> {
    let found = crate::newest_playwright_install("firefox-", "firefox/firefox");
    if found.is_none() && std::env::var_os("CHROMIUMOXIDE_REQUIRE_FIREFOX").is_some() {
        panic!("no Playwright Firefox installed (CHROMIUMOXIDE_REQUIRE_FIREFOX is set)");
    }
    found
}

fn options() -> Option<FirefoxLaunchOptions> {
    Some(FirefoxLaunchOptions {
        executable_path: Some(firefox_executable()?),
        ..Default::default()
    })
}

#[tokio::test]
async fn firefox_launches_and_runs_a_page() {
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
    let version = browser.version().await.unwrap();
    assert!(version.starts_with("159."), "{version}");
    let page = browser.new_page().await.unwrap();
    page.goto("data:text/html,<title>Fox</title><h1>Hello Firefox</h1>")
        .await
        .unwrap();
    assert_eq!(page.title().await.unwrap(), "Fox");
    assert_eq!(page.evaluate::<i64>("6 * 7").await.unwrap(), 42);
    assert_eq!(
        page.evaluate::<String>("document.querySelector('h1').textContent")
            .await
            .unwrap(),
        "Hello Firefox"
    );
    browser.close().await.unwrap();
}
