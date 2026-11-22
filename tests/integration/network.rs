//! Network events and Request/Response objects (expectations recorded from
//! Playwright 1.63 against the same server responses).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chromiumoxide::{Chromium, LaunchOptions, PageEvent};
use futures::StreamExt;

use crate::server::TestServer;

#[tokio::test]
async fn requests_and_responses_like_playwright() {
    let server = TestServer::start().await;
    server.respond("/redir", "302 Found", &[("Location", "/final")], "");
    server.respond(
        "/final",
        "200 OK",
        &[
            ("Content-Type", "text/html"),
            ("X-Custom", "yes"),
            ("Set-Cookie", "a=1"),
            ("Set-Cookie", "b=2"),
        ],
        "<p>final</p>",
    );
    server.route("/page", "text/html", "<img src=/i.png>", 0);
    let base = server.url("");

    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut events = page.events();
    let l = log.clone();
    let b = base.clone();
    tokio::spawn(async move {
        while let Some(e) = events.next().await {
            let line = match e {
                PageEvent::Request(r) => format!("request {} {}", r.method(), r.url()),
                PageEvent::Response(r) => format!("response {} {}", r.status, r.url),
                PageEvent::RequestFinished(r) => format!("finished {}", r.url()),
                PageEvent::RequestFailed(r) => {
                    format!("failed {} {}", r.url(), r.failure().unwrap_or_default())
                }
                _ => continue,
            };
            l.lock().unwrap().push(line.replace(&b, ""));
        }
    });

    // Redirect chain.
    let resp = page.goto(&server.url("/redir")).await.unwrap().unwrap();
    assert_eq!(
        (resp.url.as_str(), resp.status),
        (server.url("/final").as_str(), 200)
    );
    assert_eq!(resp.status_text, "OK");
    let req = resp.request();
    assert_eq!(req.method(), "GET");
    assert_eq!(req.resource_type(), "document");
    assert!(req.is_navigation_request());
    assert_eq!(req.frame().unwrap(), page.main_frame());
    assert_eq!(resp.frame().unwrap(), page.main_frame());
    let from = req.redirected_from().unwrap();
    assert_eq!(from.url(), server.url("/redir"));
    assert_eq!(from.redirected_to().unwrap().url(), server.url("/final"));
    assert_eq!(from.response().await.unwrap().unwrap().status, 302);
    assert!(req.headers()["user-agent"].contains("Mozilla/5.0"));

    // Headers: `headers` has no set-cookie (like Playwright), raw headers do.
    assert_eq!(resp.headers["x-custom"], "yes");
    assert!(!resp.headers.contains_key("set-cookie"));
    let all = resp.all_headers().await.unwrap();
    assert_eq!(all["set-cookie"], "a=1\nb=2");
    assert_eq!(all["x-custom"], "yes");
    assert_eq!(
        resp.header_value("Set-Cookie").await.unwrap().as_deref(),
        Some("a=1\nb=2")
    );
    assert_eq!(
        resp.header_values("set-cookie").await.unwrap(),
        ["a=1", "b=2"]
    );
    assert_eq!(resp.header_value("x-missing").await.unwrap(), None);

    // Body.
    assert_eq!(resp.text().await.unwrap(), "<p>final</p>");
    assert_eq!(resp.body().await.unwrap().len(), 12);
    assert_eq!(resp.finished().await.unwrap(), None);

    // Timing and server address.
    let timing = req.timing();
    assert!(timing.start_time > 1.0e12, "{timing:?}");
    assert!(timing.response_end >= timing.response_start, "{timing:?}");
    let addr = resp.server_addr().unwrap();
    assert_eq!(
        (addr.ip_address.as_str(), addr.port),
        ("127.0.0.1", server.port)
    );
    assert!(!resp.from_service_worker());

    // A POST fetch, matched by URL glob.
    let api = page
        .wait_for_response("**/echo")
        .run(page.evaluate::<u16>(
            "() => fetch('/echo', {method: 'POST', body: 'hi=1', headers: {'X-A': 'b'}}).then(r => r.status)",
        ))
        .await
        .unwrap();
    assert_eq!((api.status, api.status_text.as_str()), (201, "Created"));
    let api_req = api.request();
    assert_eq!(api_req.method(), "POST");
    assert_eq!(api_req.post_data().as_deref(), Some("hi=1"));
    assert_eq!(api_req.post_data_buffer().unwrap(), b"hi=1");
    assert_eq!(api_req.resource_type(), "fetch");
    assert!(!api_req.is_navigation_request());
    assert_eq!(api_req.headers()["x-a"], "b");
    let json: serde_json::Value = api.json().await.unwrap();
    assert_eq!(json, serde_json::json!({"method": "POST", "body": "hi=1"}));

    // wait_for_request with a predicate.
    let r = page
        .wait_for_request_matching(|r| r.url().ends_with("/i.png"))
        .run(page.goto(&server.url("/page")))
        .await
        .unwrap();
    assert_eq!(r.resource_type(), "image");
    assert_eq!(r.response().await.unwrap().unwrap().status, 404);

    // A failed request.
    let failed = page
        .wait_for_event(|e| match e {
            PageEvent::RequestFailed(r) => Some(r.clone()),
            _ => None,
        })
        .run(page.evaluate::<i32>("() => fetch('http://127.0.0.1:1/x').then(() => 1, () => 0)"))
        .await
        .unwrap();
    assert_eq!(failed.failure().as_deref(), Some("net::ERR_UNSAFE_PORT"));
    assert!(failed.response().await.unwrap().is_none());

    tokio::time::sleep(Duration::from_millis(300)).await;
    let log = log.lock().unwrap().clone();
    let of = |path: &str| -> Vec<String> {
        log.iter()
            .filter(|l| l.ends_with(&format!(" {path}")) || l.contains(&format!(" {path} ")))
            .cloned()
            .collect()
    };
    assert_eq!(
        of("/redir"),
        [
            "request GET /redir",
            "response 302 /redir",
            "finished /redir"
        ]
    );
    assert_eq!(
        of("/final"),
        [
            "request GET /final",
            "response 200 /final",
            "finished /final"
        ]
    );
    assert_eq!(
        of("/echo"),
        ["request POST /echo", "response 201 /echo", "finished /echo"]
    );
    assert_eq!(
        of("/i.png"),
        [
            "request GET /i.png",
            "response 404 /i.png",
            "finished /i.png"
        ]
    );
    assert_eq!(
        of("http://127.0.0.1:1/x"),
        [
            "request GET http://127.0.0.1:1/x",
            "failed http://127.0.0.1:1/x net::ERR_UNSAFE_PORT"
        ]
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn sizes_and_http_version() {
    let server = TestServer::start().await;
    server.respond(
        "/sized",
        "200 OK",
        &[("Content-Type", "text/plain")],
        "twelve bytes",
    );
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let resp = page.goto(&server.url("/sized")).await.unwrap().unwrap();
    assert_eq!(resp.http_version().await.unwrap(), "HTTP/1.1");
    let sizes = resp.request().sizes().await.unwrap();
    // The test server's exact response head.
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 12\r\nConnection: close\r\n\r\n";
    assert_eq!(sizes.response_headers_size, head.len() as u64);
    assert_eq!(sizes.response_body_size, 12);
    assert_eq!(sizes.request_body_size, 0);
    assert!(sizes.request_headers_size > 100, "{sizes:?}");
    let api = page
        .wait_for_response("**/echo")
        .run(page.evaluate::<u16>(
            "() => fetch('/echo', {method: 'POST', body: 'hi=1'}).then(r => r.status)",
        ))
        .await
        .unwrap();
    assert_eq!(api.request().sizes().await.unwrap().request_body_size, 4);
    let failed = page
        .wait_for_event(|e| match e {
            PageEvent::RequestFailed(r) => Some(r.clone()),
            _ => None,
        })
        .run(page.evaluate::<i32>("() => fetch('http://127.0.0.1:1/x').then(() => 1, () => 0)"))
        .await
        .unwrap();
    assert_eq!(
        failed.sizes().await.unwrap_err().to_string(),
        "invalid argument: Unable to fetch sizes for failed request"
    );
    browser.close().await.unwrap();
}
