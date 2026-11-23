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

/// Per-context emulation over BiDi (viewport, device scale factor, user
/// agent, locale, timezone, screen), set_content, add_style_tag, resize and
/// screenshot sizes.
#[tokio::test]
async fn firefox_contexts_emulate_devices() {
    use chromiumoxide::{ContextOptions, StyleTag, Viewport};
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
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
    page.set_viewport_size(Viewport::new(500, 400))
        .await
        .unwrap();
    assert_eq!(page.evaluate::<i64>("innerWidth").await.unwrap(), 500);

    // Not over BiDi in Firefox 159: a clear error instead of a silent no-op.
    let err = browser
        .new_context(ContextOptions {
            color_scheme: Some(chromiumoxide::ColorScheme::Dark),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Firefox"), "{err}");
    context.close().await.unwrap();
    browser.close().await.unwrap();
}
