//! Locator, page, context and browser extras (expectations recorded from
//! Playwright 1.63).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chromiumoxide::{
    Chromium, ColorScheme, ContextOptions, EmulateMedia, LaunchOptions, Locator, Media, RoleOptions,
};
use serde_json::{Value, json};

use crate::server::TestServer;

const PAGE: &str = "<h1>Title</h1><ul><li>One</li><li>Two</li></ul><button title=\"tt\">Go</button>\
<img alt=\"pic\" src=\"data:,\"><input id=t value=\"hello world\"><div hidden>h</div><div>v</div>";

#[tokio::test]
async fn locator_extras() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(PAGE).await.unwrap();
    assert_eq!(
        page.locator("ul").aria_snapshot().await.unwrap(),
        "- list:\n  - listitem: One\n  - listitem: Two"
    );
    assert_eq!(
        page.aria_snapshot().await.unwrap(),
        "- heading \"Title\" [level=1]\n- list:\n  - listitem: One\n  - listitem: Two\n- button \"Go\"\n- img \"pic\"\n- textbox: hello world\n- text: v"
    );
    let go = page.get_by_role(
        "button",
        RoleOptions {
            name: Some("Go".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        go.to_playwright_string().await.unwrap(),
        "getByRole('button', { name: 'Go' })"
    );
    assert_eq!(
        page.locator("li")
            .nth(1)
            .to_playwright_string()
            .await
            .unwrap(),
        "locator('li').nth(1)"
    );
    let described = page.locator("button").describe("the go button");
    assert_eq!(described.description().as_deref(), Some("the go button"));
    assert_eq!(
        described.to_playwright_string().await.unwrap(),
        "the go button"
    );
    assert_eq!(page.locator("li").description(), None);
    assert_eq!(
        page.locator("li")
            .evaluate_all_with::<Vec<String>, _>("(els, s) => els.map(e => e.textContent + s)", "!")
            .await
            .unwrap(),
        ["One!", "Two!"]
    );
    assert_eq!(
        page.locator("li")
            .evaluate_all::<usize>("els => els.length")
            .await
            .unwrap(),
        2
    );
    assert_eq!(page.locator("div").visible(true).count().await.unwrap(), 1);
    page.locator("#t").select_text().await.unwrap();
    assert_eq!(
        page.evaluate::<String>(
            "() => { const e = document.activeElement; return e.value.substring(e.selectionStart, e.selectionEnd); }"
        )
        .await
        .unwrap(),
        "hello world"
    );
    for selector in ["button", "text=Go"] {
        assert_eq!(
            page.locator(selector)
                .normalize()
                .await
                .unwrap()
                .to_playwright_string()
                .await
                .unwrap(),
            "getByRole('button', { name: 'Go' })"
        );
    }
    let body = page.locator("body");
    assert_eq!(body.get_by_alt_text("pic", false).count().await.unwrap(), 1);
    assert_eq!(body.get_by_title("tt", false).count().await.unwrap(), 1);
    let handle = page
        .locator("ul")
        .evaluate_handle("e => e.children")
        .await
        .unwrap();
    assert_eq!(handle.evaluate::<usize>("c => c.length").await.unwrap(), 2);
    let el = page.locator("button").element_handle().await.unwrap();
    assert!(el.as_element().is_some());
    assert_eq!(page.locator("li").element_handles().await.unwrap().len(), 2);
    page.locator("#t").fill("").await.unwrap();
    #[allow(deprecated)]
    page.locator("#t").type_text("abc").await.unwrap();
    assert_eq!(page.locator("#t").input_value().await.unwrap(), "abc");
    page.evaluate::<()>(
        "() => { setTimeout(() => document.querySelector('#t').value = 'later', 200) }",
    )
    .await
    .unwrap();
    page.locator("#t")
        .wait_for_function("e => e.value === 'later'")
        .await
        .unwrap();
    page.locator("button").highlight().await.unwrap();
    page.locator("button").hide_highlight().await.unwrap();
    page.hide_highlight().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn page_and_context_extras() {
    let server = TestServer::start().await;
    server.route("/slow", "text/html", "<p>slow</p>", 3000);
    server.route("/a", "text/html", "<a href=/b>b</a>", 0);
    server.route("/b", "text/html", "<p>b</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    assert_eq!(browser.browser_type().name(), "chromium");
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    assert_eq!(page.context().unwrap().id(), context.id());
    assert!(context.browser().is_some());

    // Default timeouts.
    context.set_default_navigation_timeout(Duration::from_millis(300));
    let err = page.goto(&server.url("/slow")).await.unwrap_err();
    assert!(
        matches!(err, chromiumoxide::Error::Timeout { ms: 300, .. }),
        "{err}"
    );
    page.set_default_navigation_timeout(Duration::from_secs(10));
    page.goto(&server.url("/a")).await.unwrap();
    context.set_default_timeout(Duration::from_millis(200));
    let p2 = context.new_page().await.unwrap();
    let err = p2.locator("#nothing").click().await.unwrap_err();
    assert!(
        matches!(err, chromiumoxide::Error::Timeout { ms: 200, .. }),
        "{err}"
    );

    // waitForNavigation.
    let resp = page
        .wait_for_navigation()
        .run(page.click("a"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resp.url, server.url("/b"));

    // emulateMedia.
    page.emulate_media(EmulateMedia {
        media: Some(Media::Print),
        color_scheme: Some(ColorScheme::Dark),
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(
        page.evaluate::<Value>(
            "() => [matchMedia('print').matches, matchMedia('(prefers-color-scheme: dark)').matches]"
        )
        .await
        .unwrap(),
        json!([true, true])
    );
    page.bring_to_front().await.unwrap();

    // Locator handlers run before actions when their locator is visible.
    page.set_content(
        "<div id=overlay style='position:fixed;inset:0;background:red'>x</div><button onclick='window.clicked=1'>B</button>",
    )
    .await
    .unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let r = runs.clone();
    page.add_locator_handler(&page.locator("#overlay"), move |l: Locator| {
        let r = r.clone();
        async move {
            r.fetch_add(1, Ordering::SeqCst);
            l.evaluate::<()>("e => e.remove()").await.unwrap();
        }
    })
    .await
    .unwrap();
    page.click("button").await.unwrap();
    assert_eq!(
        page.evaluate::<i32>("() => window.clicked").await.unwrap(),
        1
    );
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    page.remove_locator_handler(&page.locator("#overlay"))
        .await
        .unwrap();

    // Browser-level CDP session and Chrome tracing.
    let session = browser.new_browser_cdp_session().await.unwrap();
    let v = session.send("Browser.getVersion", json!({})).await.unwrap();
    assert!(v["product"].as_str().unwrap().contains("Chrome"));
    session.detach().await.unwrap();
    let page_session = context.new_cdp_session(&page).await.unwrap();
    let r = page_session
        .send(
            "Runtime.evaluate",
            json!({"expression": "1 + 1", "returnByValue": true}),
        )
        .await
        .unwrap();
    assert_eq!(r["result"]["value"], 2);
    browser.start_tracing(None, &[]).await.unwrap();
    page.evaluate::<()>("() => {}").await.unwrap();
    let trace = browser.stop_tracing().await.unwrap();
    let parsed: Value = serde_json::from_slice(&trace).unwrap();
    assert!(
        parsed["traceEvents"]
            .as_array()
            .is_some_and(|e| !e.is_empty())
    );
    browser.close().await.unwrap();
}
