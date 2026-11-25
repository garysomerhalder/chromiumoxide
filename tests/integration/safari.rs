//! Safari over WebDriver (`chromiumoxide::Safari`). On macOS: the real
//! Safari through `safaridriver` (enable it once with
//! `safaridriver --enable`). On Linux the same code drives WebKitGTK's
//! `WebKitWebDriver` with its MiniBrowser, so the WebDriver path is tested
//! against a real WebKit everywhere.

use chromiumoxide::{Safari, SafariLaunchOptions};

/// Options for this system, or `None` (the test skips) unless
/// `CHROMIUMOXIDE_REQUIRE_SAFARI` is set.
fn options() -> Option<SafariLaunchOptions> {
    let found = if cfg!(target_os = "macos") {
        std::path::Path::new("/usr/bin/safaridriver")
            .exists()
            .then(SafariLaunchOptions::default)
    } else {
        let driver = which::which("WebKitWebDriver").ok();
        let browser = [
            "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/MiniBrowser",
            "/usr/lib/aarch64-linux-gnu/webkit2gtk-4.1/MiniBrowser",
        ]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists());
        match (driver, browser) {
            (Some(driver), Some(browser)) if std::env::var_os("DISPLAY").is_some() => {
                Some(SafariLaunchOptions {
                    driver_path: Some(driver),
                    capabilities: Some(serde_json::json!({"webkitgtk:browserOptions": {
                        "binary": browser, "args": ["--automation"]}})),
                    ..Default::default()
                })
            }
            _ => None,
        }
    };
    if found.is_none() && std::env::var_os("CHROMIUMOXIDE_REQUIRE_SAFARI").is_some() {
        panic!("no Safari WebDriver here (CHROMIUMOXIDE_REQUIRE_SAFARI is set)");
    }
    found
}

#[tokio::test]
async fn safari_runs_the_page_api() {
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route(
        "/s.html",
        "text/html",
        "<title>Safari page</title><h1 id=h>Hello</h1>\
         <button id=b onclick=\"h.textContent='clicked'\">go</button>",
        0,
    );
    let safari = Safari::launch(options).await.unwrap();
    let page = safari.page();
    page.goto(&server.url("/s.html")).await.unwrap();
    assert_eq!(page.title().await.unwrap(), "Safari page");
    assert_eq!(page.evaluate::<i64>("6 * 7").await.unwrap(), 42);
    assert!(page.url().await.unwrap().ends_with("/s.html"));
    page.locator("#b").click().await.unwrap();
    page.wait_for_function("document.getElementById('h').textContent === 'clicked'")
        .await
        .unwrap();
    assert_eq!(page.locator("#h").inner_text().await.unwrap(), "clicked");
    let png = page.screenshot().await.unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    safari.close().await.unwrap();
}
