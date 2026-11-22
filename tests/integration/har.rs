//! HAR recording and replay (shape checked against Playwright 1.63 output).

use chromiumoxide::{Chromium, ContextOptions, HarNotFound, LaunchOptions, RecordHar};
use serde_json::Value;

use crate::server::TestServer;

#[tokio::test]
async fn record_and_replay_har() {
    let server = TestServer::start().await;
    server.route("/page", "text/html", "<p>page</p>", 0);
    server.route("/img.png", "image/png", vec![0u8, 1, 2, 255], 0);
    let dir = std::env::temp_dir().join(format!("chromiumoxide-har-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.har");
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            record_har: Some(RecordHar::new(&path)),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/page")).await.unwrap();
    page.evaluate::<String>(
        "() => fetch('/echo', {method: 'POST', body: 'x=1'}).then(r => r.text())",
    )
    .await
    .unwrap();
    page.evaluate::<usize>(
        "() => fetch('/img.png').then(r => r.arrayBuffer()).then(b => b.byteLength)",
    )
    .await
    .unwrap();
    context.close().await.unwrap();

    let har: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let log = &har["log"];
    assert_eq!(log["version"], "1.2");
    assert_eq!(log["creator"]["name"], "chromiumoxide");
    assert_eq!(log["browser"]["name"], "chromium");
    let pages = log["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 1);
    let entries = log["entries"].as_array().unwrap();
    let find = |p: &str| {
        entries
            .iter()
            .find(|e| e["request"]["url"] == server.url(p))
            .unwrap_or_else(|| panic!("no entry for {p}: {entries:#?}"))
    };
    let doc = find("/page");
    assert_eq!(doc["pageref"], pages[0]["id"]);
    assert_eq!(doc["request"]["method"], "GET");
    assert_eq!(doc["response"]["status"], 200);
    assert_eq!(doc["response"]["content"]["text"], "<p>page</p>");
    assert_eq!(doc["response"]["content"]["mimeType"], "text/html");
    assert_eq!(doc["response"]["content"]["size"], 11);
    assert_eq!(doc["_resourceType"], "document");
    assert!(doc["startedDateTime"].as_str().unwrap().ends_with('Z'));
    assert!(
        doc["request"]["headers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["name"]
                .as_str()
                .unwrap()
                .eq_ignore_ascii_case("user-agent"))
    );
    let api = find("/echo");
    assert_eq!(api["request"]["method"], "POST");
    assert_eq!(api["request"]["postData"]["text"], "x=1");
    assert_eq!(api["response"]["status"], 201);
    assert_eq!(
        api["response"]["content"]["text"],
        "{\"method\":\"POST\",\"body\":\"x=1\"}"
    );
    let img = find("/img.png");
    assert_eq!(img["response"]["content"]["encoding"], "base64");
    assert_eq!(img["response"]["content"]["text"], "AAEC/w==");

    // Replay: the server now answers differently, the HAR wins.
    server.route("/page", "text/html", "<p>changed</p>", 0);
    server.route("/live", "text/plain", "live", 0);
    let c2 = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let p2 = c2.new_page().await.unwrap();
    p2.route_from_har(&path).await.unwrap();
    p2.goto(&server.url("/page")).await.unwrap();
    assert_eq!(
        p2.evaluate::<String>("() => document.body.innerHTML")
            .await
            .unwrap(),
        "<p>page</p>"
    );
    assert_eq!(
        p2.evaluate::<String>(
            "() => fetch('/echo', {method: 'POST', body: 'x=1'}).then(r => r.text())"
        )
        .await
        .unwrap(),
        "{\"method\":\"POST\",\"body\":\"x=1\"}"
    );
    assert_eq!(
        p2.evaluate::<String>("() => fetch('/live').then(() => 'ok', e => e.message)")
            .await
            .unwrap(),
        "Failed to fetch"
    );
    // With notFound: fallback, unknown requests reach the network.
    let p3 = c2.new_page().await.unwrap();
    p3.route_from_har(&path)
        .not_found(HarNotFound::Fallback)
        .await
        .unwrap();
    p3.goto(&server.url("/page")).await.unwrap();
    assert_eq!(
        p3.evaluate::<String>("() => fetch('/live').then(r => r.text())")
            .await
            .unwrap(),
        "live"
    );
    let _ = std::fs::remove_dir_all(&dir);
    browser.close().await.unwrap();
}
