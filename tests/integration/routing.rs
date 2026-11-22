//! Request routing (expectations recorded from Playwright 1.63).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chromiumoxide::{Chromium, ContextOptions, LaunchOptions, PageEvent, Route};
use futures::StreamExt;
use serde_json::json;

use crate::server::TestServer;

#[tokio::test]
async fn route_fulfill_abort_continue_fallback() {
    let server = TestServer::start().await;
    server.route("/start", "text/html", "<p>start</p>", 0);
    server.route("/once", "text/html", "<p>real /once</p>", 0);
    server.route("/both", "text/html", "<p>real /both</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    context
        .route("**/ctx", |r: Route| async move {
            r.fulfill().body("from context").await.unwrap();
        })
        .await
        .unwrap();
    context
        .route("**/both", |r: Route| async move {
            r.fulfill().body("context both").await.unwrap();
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.route("**/both", |r: Route| async move {
        r.fulfill().body("page both").await.unwrap();
    })
    .await
    .unwrap();
    page.route("**/api", |r: Route| async move {
        r.fulfill().json(&json!({"a": 1})).await.unwrap();
    })
    .await
    .unwrap();
    page.route("**/abort", |r: Route| async move {
        r.abort().await.unwrap();
    })
    .await
    .unwrap();
    page.route("**/refused", |r: Route| async move {
        r.abort().error_code("connectionrefused").await.unwrap();
    })
    .await
    .unwrap();
    page.route("**/echo", |r: Route| async move {
        let mut headers = r.request().headers();
        headers.insert("x-a".into(), "over".into());
        r.continue_()
            .method("POST")
            .post_data("changed")
            .headers(headers)
            .await
            .unwrap();
    })
    .await
    .unwrap();
    let order = Arc::new(Mutex::new(Vec::new()));
    let o = order.clone();
    page.route("**/chain", move |r: Route| {
        let o = o.clone();
        async move {
            o.lock().unwrap().push("first");
            r.fulfill()
                .status(404)
                .body("first")
                .header("X-H", "v")
                .content_type("text/plain")
                .await
                .unwrap();
        }
    })
    .await
    .unwrap();
    let o = order.clone();
    page.route("**/chain", move |r: Route| {
        let o = o.clone();
        async move {
            o.lock().unwrap().push("second");
            r.fallback().await.unwrap();
        }
    })
    .await
    .unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    page.route("**/once", move |r: Route| {
        let c = c.clone();
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            r.fulfill().body("once").await.unwrap();
        }
    })
    .times(1)
    .await
    .unwrap();

    let failed = Arc::new(Mutex::new(Vec::new()));
    let f = failed.clone();
    let mut events = page.events();
    let base = server.url("");
    tokio::spawn(async move {
        while let Some(e) = events.next().await {
            if let PageEvent::RequestFailed(r) = e {
                f.lock().unwrap().push(format!(
                    "{} {}",
                    r.url().replace(&base, ""),
                    r.failure().unwrap_or_default()
                ));
            }
        }
    });

    page.goto(&server.url("/start")).await.unwrap();
    let ev = |s: &'static str| {
        let page = page.clone();
        async move { page.evaluate::<serde_json::Value>(s).await.unwrap() }
    };
    assert_eq!(
        ev("() => fetch('/api').then(async r => [r.status, r.statusText, r.headers.get('content-type'), r.headers.get('content-length'), await r.text()])").await,
        json!([200, "OK", "application/json", "7", "{\"a\":1}"])
    );
    assert_eq!(
        ev("() => fetch('/abort').then(() => 'ok', e => e.message)").await,
        json!("Failed to fetch")
    );
    assert_eq!(
        ev("() => fetch('/refused').then(() => 'ok', e => e.message)").await,
        json!("Failed to fetch")
    );
    assert_eq!(
        ev("() => fetch('/echo').then(r => r.json())").await,
        json!({"method": "POST", "body": "changed"})
    );
    assert_eq!(
        ev("() => fetch('/chain').then(async r => [r.status, r.statusText, r.headers.get('x-h'), r.headers.get('content-type'), await r.text()])").await,
        json!([404, "Not Found", "v", "text/plain", "first"])
    );
    assert_eq!(*order.lock().unwrap(), ["second", "first"]);
    assert_eq!(
        ev("() => Promise.all([fetch('/once').then(r => r.text()), fetch('/once').then(r => r.text())])").await,
        json!(["once", "<p>real /once</p>"])
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        ev("() => fetch('/ctx').then(r => r.text())").await,
        json!("from context")
    );
    assert_eq!(
        ev("() => fetch('/both').then(r => r.text())").await,
        json!("page both")
    );
    page.unroute("**/both").await.unwrap();
    assert_eq!(
        ev("() => fetch('/both').then(r => r.text())").await,
        json!("context both")
    );
    context.unroute_all().await.unwrap();
    assert_eq!(
        ev("() => fetch('/both').then(r => r.text())").await,
        json!("<p>real /both</p>")
    );

    // A navigation fulfilled by a route.
    page.route("**/doc", |r: Route| async move {
        r.fulfill()
            .body("<h1>fake</h1>")
            .content_type("text/html")
            .await
            .unwrap();
    })
    .await
    .unwrap();
    let resp = page.goto(&server.url("/doc")).await.unwrap().unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.headers["content-length"], "13");
    assert_eq!(resp.headers["content-type"], "text/html");
    assert_eq!(resp.text().await.unwrap(), "<h1>fake</h1>");
    assert_eq!(
        page.evaluate::<String>("() => document.querySelector('h1').textContent")
            .await
            .unwrap(),
        "fake"
    );

    // The route's request is the page's Request.
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = Mutex::new(Some(tx));
    page.route("**/inspect", move |r: Route| {
        let req = r.request();
        let info = (
            req.method().to_owned(),
            req.post_data(),
            req.resource_type().to_owned(),
        );
        if let Some(tx) = tx.lock().unwrap().take() {
            let _ = tx.send(info);
        }
        async move {
            r.continue_().await.unwrap();
        }
    })
    .await
    .unwrap();
    ev("() => fetch('/inspect', {method: 'PUT', body: 'zz'}).then(r => r.status)").await;
    assert_eq!(
        rx.await.unwrap(),
        ("PUT".to_owned(), Some("zz".to_owned()), "fetch".to_owned())
    );
    assert_eq!(
        *failed.lock().unwrap(),
        [
            "/abort net::ERR_FAILED",
            "/refused net::ERR_CONNECTION_REFUSED"
        ]
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn unknown_abort_code_is_rejected() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = Mutex::new(Some(tx));
    page.route("**/*", move |r: Route| {
        let tx = tx.lock().unwrap().take();
        async move {
            let err = r.abort().error_code("nope").await.unwrap_err();
            if let Some(tx) = tx {
                let _ = tx.send(err.to_string());
            }
            r.abort().await.unwrap();
        }
    })
    .await
    .unwrap();
    let _ = page.goto("http://127.0.0.1:9/x").await;
    assert!(rx.await.unwrap().contains("Unknown error code: nope"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn route_fetch_and_fulfill_with_response_or_file() {
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>hi</p>", 0);
    server.respond(
        "/api",
        "200 OK",
        &[("Content-Type", "application/json"), ("X-Orig", "yes")],
        "{\"a\":1}",
    );
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.route("**/api", |r: Route| async move {
        let resp = r.fetch().await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["x-orig"], "yes");
        let mut v: serde_json::Value = resp.json().unwrap();
        v["extra"] = json!(1);
        r.fulfill().response(&resp).json(&v).await.unwrap();
    })
    .await
    .unwrap();
    page.route("**/redirect-me", |r: Route| async move {
        let target = r.request().url().replace("/redirect-me", "/echo");
        let resp = r
            .fetch()
            .url(&target)
            .method("POST")
            .post_data("sent")
            .await
            .unwrap();
        r.fulfill().response(&resp).await.unwrap();
    })
    .await
    .unwrap();
    let dir = std::env::temp_dir().join(format!("chromiumoxide-route-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("data.css");
    std::fs::write(&file, "body{}").unwrap();
    let f = file.clone();
    page.route("**/style", move |r: Route| {
        let f = f.clone();
        async move {
            r.fulfill().path(&f).await.unwrap();
        }
    })
    .await
    .unwrap();
    page.goto(&server.url("/")).await.unwrap();
    assert_eq!(
        page.evaluate::<serde_json::Value>(
            "() => fetch('/api').then(async r => [r.status, r.headers.get('x-orig'), await r.text()])"
        )
        .await
        .unwrap(),
        json!([200, "yes", "{\"a\":1,\"extra\":1}"])
    );
    assert_eq!(
        page.evaluate::<serde_json::Value>(
            "() => fetch('/redirect-me').then(async r => [r.status, await r.json()])"
        )
        .await
        .unwrap(),
        json!([201, {"method": "POST", "body": "sent"}])
    );
    assert_eq!(
        page.evaluate::<serde_json::Value>(
            "() => fetch('/style').then(async r => [r.headers.get('content-type'), r.headers.get('content-length'), await r.text()])"
        )
        .await
        .unwrap(),
        json!(["text/css", "6", "body{}"])
    );
    let _ = std::fs::remove_dir_all(&dir);
    browser.close().await.unwrap();
}
