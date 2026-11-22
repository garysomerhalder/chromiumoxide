//! `page.coverage` (expected entries recorded from Playwright 1.63).

use chromiumoxide::{Chromium, LaunchOptions};
use serde_json::json;

use crate::server::TestServer;

#[tokio::test]
async fn js_and_css_coverage() {
    let server = TestServer::start().await;
    server.route(
        "/p.html",
        "text/html",
        "<link rel=stylesheet href=\"/s.css\"><div class=used>x</div><script src=\"/a.js\"></script>",
        0,
    );
    server.route(
        "/s.css",
        "text/css",
        ".used { color: red; }\n.unused { color: blue; }\n",
        0,
    );
    server.route(
        "/a.js",
        "text/javascript",
        "function used() { return 1; }\nfunction unused() { return 2; }\nused();\n",
        0,
    );
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let coverage = page.coverage();
    coverage
        .start_js_coverage(Default::default())
        .await
        .unwrap();
    coverage
        .start_css_coverage(Default::default())
        .await
        .unwrap();
    let err = coverage
        .start_js_coverage(Default::default())
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("JSCoverage is already enabled"),
        "{err}"
    );
    page.goto(&server.url("/p.html")).await.unwrap();
    let js = coverage.stop_js_coverage().await.unwrap();
    let css = coverage.stop_css_coverage().await.unwrap();

    assert_eq!(js.len(), 1);
    assert_eq!(js[0].url, server.url("/a.js"));
    assert_eq!(
        js[0].source.as_deref(),
        Some("function used() { return 1; }\nfunction unused() { return 2; }\nused();\n")
    );
    assert_eq!(
        js[0].functions,
        json!([{"functionName":"","isBlockCoverage":true,"ranges":[{"startOffset":0,"endOffset":70,"count":1}]},
               {"functionName":"used","isBlockCoverage":true,"ranges":[{"startOffset":0,"endOffset":29,"count":1}]},
               {"functionName":"unused","isBlockCoverage":false,"ranges":[{"startOffset":30,"endOffset":61,"count":0}]}])
    );
    assert_eq!(css.len(), 1);
    assert_eq!(css[0].url, server.url("/s.css"));
    assert_eq!(
        css[0].text.as_deref(),
        Some(".used { color: red; }\n.unused { color: blue; }\n")
    );
    assert_eq!(
        css[0].ranges,
        [chromiumoxide::CoverageRange { start: 0, end: 21 }]
    );
    // Stopping again reports nothing.
    assert!(coverage.stop_js_coverage().await.unwrap().is_empty());
    browser.close().await.unwrap();
}
