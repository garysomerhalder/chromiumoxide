//! Context-level events (`context.on('request' | 'response' | 'console' |
//! 'weberror' | 'dialog')`) and service-worker requests.

use std::time::Duration;

use chromiumoxide::{Chromium, ContextEvent, ContextOptions, LaunchOptions};
use futures::StreamExt;

use crate::server::TestServer;

#[tokio::test]
async fn context_reports_page_events() {
    let server = TestServer::start().await;
    server.route("/p.html", "text/html", "<p>p</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let mut events = ctx.events();
    let page = ctx.new_page().await.unwrap();
    page.goto(&server.url("/p.html")).await.unwrap();
    page.evaluate::<()>(
        "() => { console.log('hello'); setTimeout(() => { throw new Error('boom'); }); }",
    )
    .await
    .unwrap();
    let (mut request, mut response, mut finished, mut console, mut error) =
        (false, false, false, false, false);
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(e) = events.next().await {
            match e {
                ContextEvent::Request(r) if r.url() == server.url("/p.html") => request = true,
                ContextEvent::Response(r) if r.url == server.url("/p.html") => response = true,
                ContextEvent::RequestFinished(r) if r.url() == server.url("/p.html") => {
                    finished = true
                }
                ContextEvent::Console(m) if m.text() == "hello" => {
                    assert_eq!(m.page().as_ref(), Some(&page));
                    console = true
                }
                ContextEvent::WebError(e) => {
                    assert!(e.error.message.contains("boom"), "{e:?}");
                    error = true
                }
                _ => {}
            }
            if request && response && finished && console && error {
                break;
            }
        }
    })
    .await
    .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn service_worker_requests_carry_their_worker() {
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>sw</p>", 0);
    server.route("/from-sw", "text/plain", "data", 0);
    server.route(
        "/sw.js",
        "text/javascript",
        "self.addEventListener('install', e => e.waitUntil(fetch('/from-sw')));",
        0,
    );
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let mut events = ctx.events();
    let page = ctx.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    page.evaluate::<()>("async () => { await navigator.serviceWorker.register('/sw.js'); }")
        .await
        .unwrap();
    let (req, resp) = tokio::time::timeout(Duration::from_secs(10), async {
        let mut req = None;
        while let Some(e) = events.next().await {
            match e {
                ContextEvent::Request(r) if r.url() == server.url("/from-sw") => req = Some(r),
                ContextEvent::Response(r) if r.url == server.url("/from-sw") => {
                    return (req.expect("request first"), r);
                }
                _ => {}
            }
        }
        panic!("no service worker request")
    })
    .await
    .unwrap();
    let worker = req.service_worker().expect("made by the service worker");
    assert_eq!(worker.url(), server.url("/sw.js"));
    assert!(req.frame().is_none());
    assert_eq!(resp.status, 200);
    assert_eq!(resp.text().await.unwrap(), "data");
    browser.close().await.unwrap();
}
