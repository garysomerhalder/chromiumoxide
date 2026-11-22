use std::time::Duration;

use chromiumoxide::{Chromium, Error, LaunchOptions, UrlMatcher, WaitUntil};

use crate::server::TestServer;

#[tokio::test]
async fn reload_back_and_forward() {
    let server = TestServer::start().await;
    server.route(
        "/a",
        "text/html",
        "<title>A</title><script>window.n = (window.n || 0) + 1</script>",
        0,
    );
    server.route("/b", "text/html", "<title>B</title>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/a")).await.unwrap();
    page.evaluate::<()>("() => { window.marker = 1 }")
        .await
        .unwrap();
    let r = page.reload().await.unwrap().unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(
        page.evaluate::<Option<i64>>("window.marker").await.unwrap(),
        None,
        "fresh document"
    );

    page.goto(&server.url("/b")).await.unwrap();
    let back = page.go_back().await.unwrap();
    assert!(back.is_some());
    assert_eq!(page.url(), server.url("/a"));
    assert_eq!(page.title().await.unwrap(), "A");
    page.go_forward()
        .wait_until(WaitUntil::DomContentLoaded)
        .await
        .unwrap();
    assert_eq!(page.url(), server.url("/b"));
    // nothing further: Playwright returns null
    assert!(page.go_forward().await.unwrap().is_none());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn wait_for_url_glob_regex_and_timeout() {
    let server = TestServer::start().await;
    server.route(
        "/start",
        "text/html",
        "<script>setTimeout(() => location.href = '/done?x=1', 150)</script>",
        0,
    );
    server.route("/done", "text/html", "done", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/start")).await.unwrap();
    page.wait_for_url(UrlMatcher::glob("**/done?*"))
        .await
        .unwrap();
    assert!(page.url().ends_with("/done?x=1"));
    page.wait_for_url(UrlMatcher::regex(r"/done\?x=\d$").unwrap())
        .await
        .unwrap();
    page.wait_for_url(UrlMatcher::exact(&server.url("/done?x=1")))
        .await
        .unwrap();
    let err = page
        .wait_for_url(UrlMatcher::glob("**/never"))
        .timeout(Duration::from_millis(200))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Timeout { ms: 200, .. }), "{err:?}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn wait_for_timeout_waits() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let start = std::time::Instant::now();
    page.wait_for_timeout(Duration::from_millis(120)).await;
    assert!(start.elapsed() >= Duration::from_millis(120));
    browser.close().await.unwrap();
}
