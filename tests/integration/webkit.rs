//! The WebKit engine (`chromiumoxide::webkit`), against Playwright's WebKit
//! build (`webkit-<revision>/pw_run.sh`), over its `--inspector-pipe`.

use chromiumoxide::WebKit;
use chromiumoxide::webkit::WebKitLaunchOptions;

/// Playwright's WebKit, or `None` (tests skip) unless
/// `CHROMIUMOXIDE_REQUIRE_WEBKIT` is set.
pub fn webkit_executable() -> Option<std::path::PathBuf> {
    let exe = if cfg!(windows) {
        "Playwright.exe"
    } else {
        "pw_run.sh"
    };
    let found = crate::newest_playwright_install("webkit-", exe);
    if found.is_none() && std::env::var_os("CHROMIUMOXIDE_REQUIRE_WEBKIT").is_some() {
        panic!("no Playwright WebKit installed (CHROMIUMOXIDE_REQUIRE_WEBKIT is set)");
    }
    found
}

pub fn options() -> Option<WebKitLaunchOptions> {
    Some(WebKitLaunchOptions {
        executable_path: Some(webkit_executable()?),
        ..Default::default()
    })
}

#[tokio::test]
async fn webkit_launches_and_runs_a_page() {
    let Some(options) = options() else {
        return;
    };
    let browser = WebKit::launch(options).await.unwrap();
    assert!(!browser.version().await.unwrap().is_empty());
    let page = browser.new_page().await.unwrap();
    page.goto("data:text/html,<title>Kit</title><h1>Hello WebKit</h1>")
        .await
        .unwrap();
    assert_eq!(page.title().await.unwrap(), "Kit");
    assert_eq!(page.evaluate::<i64>("6 * 7").await.unwrap(), 42);
    assert_eq!(
        page.evaluate::<String>("document.querySelector('h1').textContent")
            .await
            .unwrap(),
        "Hello WebKit"
    );
    // Playwright's default user agent for its WebKit.
    let ua: String = page.evaluate("navigator.userAgent").await.unwrap();
    assert!(
        ua.contains("AppleWebKit/605.1.15") && ua.contains("Safari"),
        "{ua}"
    );
    browser.close().await.unwrap();
}

/// Context emulation, set_content, add_style_tag, resize and screenshots.
#[tokio::test]
async fn webkit_contexts_emulate_devices() {
    use chromiumoxide::{ContextOptions, StyleTag, Viewport};
    let Some(options) = options() else {
        return;
    };
    let browser = WebKit::launch(options).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(390, 664)),
            screen: Some(Viewport::new(390, 844)),
            device_scale_factor: Some(3.0),
            user_agent: Some("CoxTest/1.0".into()),
            locale: Some("en-GB".into()),
            timezone_id: Some("Asia/Tokyo".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.set_content("<p id=p>hi</p>").await.unwrap();
    let seen: serde_json::Value = page
        .evaluate(
            "({ w: innerWidth, h: innerHeight, dpr: devicePixelRatio, ua: navigator.userAgent,
               lang: navigator.language, tz: Intl.DateTimeFormat().resolvedOptions().timeZone,
               sw: screen.width, sh: screen.height })",
        )
        .await
        .unwrap();
    assert_eq!(
        seen,
        serde_json::json!({"w": 390, "h": 664, "dpr": 3, "ua": "CoxTest/1.0", "lang": "en-GB",
                           "tz": "Asia/Tokyo", "sw": 390, "sh": 844})
    );
    page.add_style_tag(StyleTag::content("#p { color: rgb(1, 2, 3) }"))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<String>("getComputedStyle(document.getElementById('p')).color")
            .await
            .unwrap(),
        "rgb(1, 2, 3)"
    );
    let png = page.screenshot().await.unwrap();
    let (w, h, _) = crate::screenshot::decode(&png);
    assert_eq!((w, h), (390 * 3, 664 * 3));
    let full = page.screenshot().full_page(true).await.unwrap();
    assert_eq!(crate::screenshot::decode(&full).0, 390 * 3);
    page.set_viewport_size(Viewport::new(500, 400))
        .await
        .unwrap();
    assert_eq!(page.evaluate::<i64>("innerWidth").await.unwrap(), 500);
    page.wait_for_function("innerWidth === 500").await.unwrap();
    context.close().await.unwrap();
    browser.close().await.unwrap();
}
