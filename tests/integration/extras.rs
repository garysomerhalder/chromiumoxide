//! File chooser, history helpers and other M4 odds and ends (expectations
//! recorded from Playwright 1.63).

use chromiumoxide::{Chromium, ContextOptions, LaunchOptions, NewCookie, StorageState};
use serde_json::{Value, json};

use crate::server::TestServer;

#[tokio::test]
async fn file_chooser_like_playwright() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content("<input type=file id=f multiple><input type=file id=s>")
        .await
        .unwrap();
    let chooser = page
        .wait_for_file_chooser()
        .run(page.click("#f"))
        .await
        .unwrap();
    assert!(chooser.is_multiple());
    assert_eq!(chooser.page(), page);
    assert_eq!(
        chooser
            .element()
            .await
            .unwrap()
            .evaluate::<String>("e => e.id")
            .await
            .unwrap(),
        "f"
    );
    let dir = std::env::temp_dir().join(format!("chromiumoxide-fc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.txt");
    std::fs::write(&file, "hi").unwrap();
    chooser.set_files(&[file]).await.unwrap();
    assert_eq!(
        page.evaluate::<Value>(
            "() => [...document.querySelector('#f').files].map(f => f.name + ':' + f.size)"
        )
        .await
        .unwrap(),
        json!(["a.txt:2"])
    );
    let single = page
        .wait_for_file_chooser()
        .run(page.click("#s"))
        .await
        .unwrap();
    assert!(!single.is_multiple());
    let _ = std::fs::remove_dir_all(&dir);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn histories_requests_and_console_details() {
    let server = TestServer::start().await;
    server.route("/a", "text/html", "<p>a</p>", 0);
    server.route("/b", "text/plain", "b", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();

    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let msg = page
        .wait_for_console_message(|_| true)
        .run(page.evaluate::<()>("() => console.log('t')"))
        .await
        .unwrap();
    assert_eq!(msg.page(), Some(page.clone()));
    assert!(msg.worker().is_none());
    assert!(
        msg.timestamp() >= before - 1000.0 && msg.timestamp() < before + 60_000.0,
        "{}",
        msg.timestamp()
    );
    assert_eq!(page.console_messages().len(), 1);
    page.clear_console_messages();
    assert!(page.console_messages().is_empty());
    page.wait_for_page_error()
        .run(page.evaluate::<()>("() => { setTimeout(() => { throw new Error('x') }) }"))
        .await
        .unwrap();
    assert_eq!(page.page_errors().len(), 1);
    page.clear_page_errors();
    assert!(page.page_errors().is_empty());

    page.goto(&server.url("/a")).await.unwrap();
    page.evaluate::<String>("() => fetch('/b').then(r => r.text())")
        .await
        .unwrap();
    let urls: Vec<String> = page
        .requests()
        .iter()
        .map(|r| r.url().replace(&server.url(""), ""))
        .collect();
    assert_eq!(urls, ["/a", "/b"]);
    let doc = page.requests()[0].clone();
    assert_eq!(doc.existing_response().unwrap().status, 200);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn set_storage_state_replaces_cookies_and_storage() {
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>hi</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    page.evaluate::<()>("() => localStorage.setItem('old', '1')")
        .await
        .unwrap();
    context
        .add_cookies(&[NewCookie::new("old", "1").url(&server.url("/"))])
        .await
        .unwrap();
    let state: StorageState = serde_json::from_value(json!({
        "cookies": [{"name": "new", "value": "2", "domain": "127.0.0.1", "path": "/",
                     "expires": -1, "httpOnly": false, "secure": false, "sameSite": "Lax"}],
        "origins": [{"origin": server.url(""), "localStorage": [{"name": "fresh", "value": "3"}]}]
    }))
    .unwrap();
    context.set_storage_state(&state).await.unwrap();
    let names: Vec<String> = context
        .cookies()
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["new"]);
    let p2 = context.new_page().await.unwrap();
    p2.goto(&server.url("/")).await.unwrap();
    assert_eq!(
        p2.evaluate::<Value>("() => [localStorage.getItem('old'), localStorage.getItem('fresh')]")
            .await
            .unwrap(),
        json!([null, "3"])
    );
    browser.close().await.unwrap();
}
