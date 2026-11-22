use std::time::Duration;

use chromiumoxide::{Chromium, Error, LaunchOptions, Page, Polling, StyleTag, Viewport, WaitUntil};

use crate::server::TestServer;

async fn page() -> (chromiumoxide::Browser, Page) {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    (browser, page)
}

const PIXEL_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0, 0, 0,
    1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, 0x49, 0x44, 0x41, 0x54,
    0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0, 0x05, 0, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d,
    0, 0, 0, 0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[tokio::test]
async fn goto_waits_for_load_including_images() {
    let server = TestServer::start().await;
    server.route("/img.png", "image/png", PIXEL_PNG, 400);
    server.route("/", "text/html", "<img src='/img.png'>", 0);
    let (browser, page) = page().await;
    let response = page.goto(&server.url("/")).await.unwrap().unwrap();
    assert_eq!(response.status, 200);
    assert!(response.ok());
    assert_eq!(response.url, server.url("/"));
    assert!(
        page.evaluate::<bool>("document.images[0].complete && document.readyState === 'complete'")
            .await
            .unwrap()
    );
    assert_eq!(page.url(), server.url("/"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn goto_domcontentloaded_does_not_wait_for_images() {
    let server = TestServer::start().await;
    server.route("/img.png", "image/png", PIXEL_PNG, 1500);
    server.route("/", "text/html", "<img src='/img.png'>", 0);
    let (browser, page) = page().await;
    page.goto(&server.url("/"))
        .wait_until(WaitUntil::DomContentLoaded)
        .await
        .unwrap();
    assert!(
        !page
            .evaluate::<bool>("document.images[0].complete")
            .await
            .unwrap()
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn goto_networkidle_waits_for_late_requests() {
    let server = TestServer::start().await;
    server.route("/slow", "text/plain", "ok", 400);
    server.route(
        "/",
        "text/html",
        "<script>addEventListener('load', () => setTimeout(() => fetch('/slow').then(r => r.text()).then(t => window.done = t), 100))</script>",
        0,
    );
    let (browser, page) = page().await;
    page.goto(&server.url("/"))
        .wait_until(WaitUntil::NetworkIdle)
        .await
        .unwrap();
    assert_eq!(page.evaluate::<String>("window.done").await.unwrap(), "ok");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn goto_reports_network_errors_and_data_urls_have_no_response() {
    let (browser, page) = page().await;
    let err = page.goto("http://127.0.0.1:9/").await.unwrap_err();
    assert!(matches!(err, Error::Navigation(_)), "{err:?}");
    assert!(err.to_string().contains("net::ERR_"), "{err}");
    let r = page.goto("data:text/html,<h1>hi</h1>").await.unwrap();
    assert!(r.is_none());
    assert_eq!(
        page.evaluate::<String>("document.querySelector('h1').textContent")
            .await
            .unwrap(),
        "hi"
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn goto_times_out() {
    let server = TestServer::start().await;
    server.route("/", "text/html", "slow", 3000);
    let (browser, page) = page().await;
    let err = page
        .goto(&server.url("/"))
        .timeout(Duration::from_millis(300))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Timeout { ms: 300, .. }), "{err:?}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn set_content_waits_for_load_images_and_fonts() {
    let server = TestServer::start().await;
    server.route("/img.png", "image/png", PIXEL_PNG, 400);
    let (browser, page) = page().await;
    let html = format!(
        "<style>body{{margin:0}}</style><img src='{}'><p>text</p>",
        server.url("/img.png")
    );
    page.set_content(&html).await.unwrap();
    let ready: serde_json::Value = page
        .evaluate("() => [document.readyState, document.images[0].complete, document.fonts.status, document.querySelector('p').textContent]")
        .await
        .unwrap();
    assert_eq!(
        ready,
        serde_json::json!(["complete", true, "loaded", "text"])
    );
    // a second setContent replaces the first and waits again
    page.set_content("<p id=two>2</p>").await.unwrap();
    assert_eq!(
        page.evaluate::<String>("document.getElementById('two').textContent")
            .await
            .unwrap(),
        "2"
    );
    assert!(
        page.content()
            .await
            .unwrap()
            .contains("<p id=\"two\">2</p>")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn set_content_title_and_content() {
    let (browser, page) = page().await;
    page.set_content("<title>Hello</title><div>x</div>")
        .await
        .unwrap();
    assert_eq!(page.title().await.unwrap(), "Hello");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn wait_for_function_polls_until_truthy() {
    let (browser, page) = page().await;
    page.evaluate::<()>("() => { setTimeout(() => window.ready = 42, 200) }")
        .await
        .unwrap();
    let v = page.wait_for_function("() => window.ready").await.unwrap();
    assert_eq!(v, 42);
    let v = page
        .wait_for_function("(n) => window.ready === n && 'yes'")
        .arg(42)
        .polling(Polling::Interval(Duration::from_millis(50)))
        .await
        .unwrap();
    assert_eq!(v, "yes");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn wait_for_function_times_out_and_propagates_errors() {
    let (browser, page) = page().await;
    let err = page
        .wait_for_function("() => false")
        .timeout(Duration::from_millis(200))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Timeout { ms: 200, .. }), "{err:?}");
    let err = page
        .wait_for_function("() => { throw new Error('bad predicate') }")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("bad predicate"), "{err}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn wait_for_function_survives_navigation() {
    let server = TestServer::start().await;
    server.route(
        "/next",
        "text/html",
        "<script>window.arrived = true</script>",
        0,
    );
    let (browser, page) = page().await;
    let url = server.url("/next");
    let waiter = {
        let page = page.clone();
        tokio::spawn(async move { page.wait_for_function("() => window.arrived").await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    page.goto(&url).await.unwrap();
    assert_eq!(waiter.await.unwrap().unwrap(), true);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn add_style_tag_content_and_url() {
    let server = TestServer::start().await;
    server.route("/style.css", "text/css", "p { margin-left: 7px }", 200);
    let (browser, page) = page().await;
    page.set_content("<p>x</p>").await.unwrap();
    page.add_style_tag(StyleTag::content("p { color: rgb(1, 2, 3) }"))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<String>("getComputedStyle(document.querySelector('p')).color")
            .await
            .unwrap(),
        "rgb(1, 2, 3)"
    );
    page.add_style_tag(StyleTag::url(&server.url("/style.css")))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<String>("getComputedStyle(document.querySelector('p')).marginLeft")
            .await
            .unwrap(),
        "7px"
    );
    let err = page
        .add_style_tag(StyleTag::url(&server.url("/missing.css")))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Failed to load"), "{err}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn set_viewport_size_resizes() {
    let (browser, page) = page().await;
    page.set_viewport_size(Viewport::new(500, 400))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<Vec<u32>>("[innerWidth, innerHeight]")
            .await
            .unwrap(),
        vec![500, 400]
    );
    assert_eq!(page.viewport_size(), Some(Viewport::new(500, 400)));
    browser.close().await.unwrap();
}
