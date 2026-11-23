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

/// Locators run Playwright's injected script in a BiDi sandbox: CSS, role,
/// text and test-id selectors, nth/first/last, chaining, reads, visibility,
/// waiting for late elements and strictness.
#[tokio::test]
async fn firefox_locators_query_the_page() {
    use chromiumoxide::RoleOptions;
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<ul><li class=a>One</li><li class=a>Two</li><li class=a data-testid=three>Three</li></ul>
         <button title=go>Go <b>now</b></button><input hidden value=x>
         <section><p>inner</p></section>
         <script>setTimeout(() => document.body.insertAdjacentHTML('beforeend', '<p id=late>Late</p>'), 300)</script>",
    )
    .await
    .unwrap();
    let items = page.locator("li.a");
    assert_eq!(items.count().await.unwrap(), 3);
    assert_eq!(
        items.all_text_contents().await.unwrap(),
        ["One", "Two", "Three"]
    );
    assert_eq!(
        items.nth(1).text_content().await.unwrap().as_deref(),
        Some("Two")
    );
    assert_eq!(items.first().inner_text().await.unwrap(), "One");
    assert_eq!(items.last().inner_text().await.unwrap(), "Three");
    assert_eq!(
        page.get_by_role("button", RoleOptions::default())
            .inner_text()
            .await
            .unwrap(),
        "Go now"
    );
    assert_eq!(
        page.locator("button")
            .get_attribute("title")
            .await
            .unwrap()
            .as_deref(),
        Some("go")
    );
    assert_eq!(
        page.get_by_test_id("three")
            .text_content()
            .await
            .unwrap()
            .as_deref(),
        Some("Three")
    );
    assert!(page.get_by_text("Two", true).is_visible().await.unwrap());
    assert!(!page.locator("input").is_visible().await.unwrap());
    assert_eq!(
        page.locator("section")
            .locator("p")
            .inner_html()
            .await
            .unwrap(),
        "inner"
    );
    // Reads wait for the element, like Playwright.
    assert_eq!(page.locator("#late").inner_text().await.unwrap(), "Late");
    let err = page.locator("li").text_content().await.unwrap_err();
    assert!(err.to_string().contains("strict mode violation"), "{err}");
    browser.close().await.unwrap();
}

/// Input over BiDi `input.performActions`: click, fill, press (named keys
/// and chords), press_sequentially, check/uncheck, hover.
#[tokio::test]
async fn firefox_locators_act_on_elements() {
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<div style='height: 1500px'>spacer</div>
         <button onclick=\"this.textContent = 'clicked ' + event.detail\">Go</button>
         <input id=i><input type=checkbox id=c>
         <div id=h onmouseover=\"this.textContent = 'hovered'\">hover me</div>
         <input id=k onkeydown=\"this.dataset.keys = (this.dataset.keys || '') + event.key + ','\">",
    )
    .await
    .unwrap();
    // Below the fold: click scrolls it into view first.
    page.locator("button").click().await.unwrap();
    assert_eq!(
        page.locator("button").inner_text().await.unwrap(),
        "clicked 1"
    );

    let input = page.locator("#i");
    input.fill("hello").await.unwrap();
    assert_eq!(input.input_value().await.unwrap(), "hello");
    input.press("Backspace").await.unwrap();
    assert_eq!(input.input_value().await.unwrap(), "hell");
    input.press("Control+a").await.unwrap();
    input.press_sequentially("yo!").await.unwrap();
    assert_eq!(input.input_value().await.unwrap(), "yo!");
    input.fill("").await.unwrap();
    assert_eq!(input.input_value().await.unwrap(), "");

    let check = page.locator("#c");
    check.check().await.unwrap();
    assert!(
        page.evaluate::<bool>("document.getElementById('c').checked")
            .await
            .unwrap()
    );
    check.uncheck().await.unwrap();
    assert!(
        !page
            .evaluate::<bool>("document.getElementById('c').checked")
            .await
            .unwrap()
    );

    page.locator("#h").hover().await.unwrap();
    assert_eq!(page.locator("#h").inner_text().await.unwrap(), "hovered");

    page.locator("#k").press("Enter").await.unwrap();
    page.locator("#k").press("ArrowLeft").await.unwrap();
    assert_eq!(
        page.locator("#k")
            .get_attribute("data-keys")
            .await
            .unwrap()
            .as_deref(),
        Some("Enter,ArrowLeft,")
    );
    browser.close().await.unwrap();
}
