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

#[tokio::test]
async fn page_web_storage_helpers() {
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>x</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    page.local_storage().set_item("a", "1").await.unwrap();
    page.session_storage().set_item("s", "2").await.unwrap();
    let items = page.local_storage().items().await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        (items[0].name.as_str(), items[0].value.as_str()),
        ("a", "1")
    );
    assert_eq!(
        page.local_storage().get_item("a").await.unwrap().as_deref(),
        Some("1")
    );
    assert_eq!(page.local_storage().get_item("zz").await.unwrap(), None);
    assert_eq!(
        page.evaluate::<Value>("() => [localStorage.a, sessionStorage.s]")
            .await
            .unwrap(),
        json!(["1", "2"])
    );
    page.local_storage().remove_item("a").await.unwrap();
    page.session_storage().clear().await.unwrap();
    assert!(page.local_storage().items().await.unwrap().is_empty());
    assert!(page.session_storage().items().await.unwrap().is_empty());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn service_workers_are_reported() {
    use chromiumoxide::ContextEvent;
    use futures::StreamExt;
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>sw</p>", 0);
    server.route("/sw.js", "text/javascript", "self.x = 5;", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let mut events = context.events();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    page.evaluate::<()>("async () => { await navigator.serviceWorker.register('/sw.js'); }")
        .await
        .unwrap();
    let sw = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(e) = events.next().await {
            if let ContextEvent::ServiceWorker(w) = e {
                return w;
            }
        }
        panic!("no service worker")
    })
    .await
    .unwrap();
    assert_eq!(sw.url(), server.url("/sw.js"));
    assert_eq!(context.service_workers().len(), 1);
    assert_eq!(sw.evaluate::<i32>("() => self.x").await.unwrap(), 5);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn download_bytes_can_be_read() {
    let server = TestServer::start().await;
    server.respond(
        "/dl",
        "200 OK",
        &[("Content-Disposition", "attachment; filename=f.txt")],
        "abc",
    );
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(&format!("<a href='{}'>dl</a>", server.url("/dl")))
        .await
        .unwrap();
    let download = page.wait_for_download().run(page.click("a")).await.unwrap();
    assert_eq!(download.read().await.unwrap(), b"abc");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn webauthn_credentials() {
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>webauthn</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let credentials = context.credentials();
    credentials.install().await.unwrap();
    let made = credentials
        .create(chromiumoxide::NewCredential::new("localhost"))
        .await
        .unwrap();
    assert_eq!(made.rp_id, "localhost");
    assert!(!made.id.is_empty());
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/").replace("127.0.0.1", "localhost"))
        .await
        .unwrap();
    // The page registers a passkey through the virtual authenticator.
    let id: String = page
        .evaluate(
            "async () => { const c = await navigator.credentials.create({publicKey: { \
             challenge: new Uint8Array([1,2,3]), rp: {name: 'x', id: 'localhost'}, \
             user: {id: new Uint8Array([7]), name: 'u', displayName: 'U'}, \
             pubKeyCredParams: [{type: 'public-key', alg: -7}], \
             authenticatorSelection: {residentKey: 'required', userVerification: 'preferred'} }}); return c.id; }",
        )
        .await
        .unwrap();
    assert!(!id.is_empty());
    let all = credentials.get(None, None).await.unwrap();
    assert_eq!(all.len(), 2, "{all:?}");
    assert!(all.iter().any(|c| c.id == id));
    assert_eq!(
        credentials
            .get(Some("other.test"), None)
            .await
            .unwrap()
            .len(),
        0
    );
    credentials.delete(&made.id).await.unwrap();
    assert_eq!(credentials.get(None, None).await.unwrap().len(), 1);
    browser.close().await.unwrap();
}

/// Playwright 1.64's `signCount`: set on create, returned by create/get and
/// advanced by each sign-in through the authenticator.
#[tokio::test]
async fn webauthn_sign_count() {
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>webauthn</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let credentials = context.credentials();
    credentials.install().await.unwrap();
    let made = credentials
        .create(chromiumoxide::NewCredential {
            sign_count: Some(5),
            ..chromiumoxide::NewCredential::new("localhost")
        })
        .await
        .unwrap();
    assert_eq!(made.sign_count, 5);
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/").replace("127.0.0.1", "localhost"))
        .await
        .unwrap();
    page.evaluate::<String>(
        "async () => { const a = await navigator.credentials.get({publicKey: { \
         challenge: new Uint8Array([1,2,3]), rpId: 'localhost', userVerification: 'preferred' }}); return a.id; }",
    )
    .await
    .unwrap();
    let got = credentials.get(None, Some(&made.id)).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].sign_count, 6, "{got:?}");
    browser.close().await.unwrap();
}

/// Playwright 1.65's `cdpSession.send(method, params, { timeout })`.
#[tokio::test]
async fn cdp_session_send_with_timeout() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let session = page
        .context()
        .unwrap()
        .new_cdp_session(&page)
        .await
        .unwrap();
    let v = session
        .send_with_timeout(
            "Runtime.evaluate",
            serde_json::json!({"expression": "1 + 2", "returnByValue": true}),
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(v["result"]["value"], 3);
    let err = session
        .send_with_timeout(
            "Runtime.evaluate",
            serde_json::json!({"expression": "new Promise(() => {})", "awaitPromise": true}),
            std::time::Duration::from_millis(300),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, chromiumoxide::Error::Timeout { .. }),
        "{err:?}"
    );
    browser.close().await.unwrap();
}
