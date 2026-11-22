//! WebSockets and web workers (expectations recorded from Playwright 1.63).

use std::time::Duration;

use async_tungstenite::tungstenite::Message;
use chromiumoxide::{Chromium, LaunchOptions, PageEvent, WebSocketEvent, WsPayload};
use futures::{SinkExt, StreamExt};

use crate::server::TestServer;

/// Echoes `echo:<text>` plus a binary [1, 2, 3] for each text message.
async fn ws_server() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(mut ws) = async_tungstenite::tokio::accept_async(sock).await else {
                    return;
                };
                while let Some(Ok(msg)) = ws.next().await {
                    match msg {
                        Message::Text(t) => {
                            let _ = ws.send(Message::text(format!("echo:{t}"))).await;
                            let _ = ws.send(Message::binary(vec![1u8, 2, 3])).await;
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            });
        }
    });
    port
}

#[tokio::test]
async fn websockets_like_playwright() {
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>ws</p>", 0);
    let port = ws_server().await;
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    let ws = page
        .wait_for_websocket()
        .run(page.evaluate::<()>(&format!(
            "() => {{ const ws = new WebSocket('ws://127.0.0.1:{port}/sock'); ws.onopen = () => ws.send('hello'); \
             let n = 0; ws.onmessage = () => {{ if (++n === 2) ws.close(); }}; }}"
        )))
        .await
        .unwrap();
    assert_eq!(ws.url(), format!("ws://127.0.0.1:{port}/sock"));
    // Events replay from the start, so nothing is missed after the wait.
    let mut events = ws.events();
    let mut log = Vec::new();
    while let Some(e) = tokio::time::timeout(Duration::from_secs(10), events.next())
        .await
        .unwrap()
    {
        match e {
            WebSocketEvent::FrameSent(WsPayload::Text(t)) => log.push(format!("sent {t}")),
            WebSocketEvent::FrameReceived(WsPayload::Text(t)) => log.push(format!("recv {t}")),
            WebSocketEvent::FrameReceived(WsPayload::Binary(b)) => log.push(format!("recv {b:?}")),
            WebSocketEvent::Close => {
                log.push("close".into());
                break;
            }
            other => log.push(format!("{other:?}")),
        }
    }
    assert_eq!(
        log,
        ["sent hello", "recv echo:hello", "recv [1, 2, 3]", "close"]
    );
    assert!(ws.is_closed());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn workers_like_playwright() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let mut events = page.events();
    let worker = page
        .wait_for_worker()
        .run(page.evaluate::<()>(
            "() => { window.w = new Worker(URL.createObjectURL(new Blob(['self.x = 7; console.log(\"from worker\")'], {type: 'application/javascript'}))); }",
        ))
        .await
        .unwrap();
    assert!(worker.url().starts_with("blob:"), "{}", worker.url());
    assert_eq!(page.workers().len(), 1);
    assert_eq!(worker.evaluate::<i32>("() => self.x").await.unwrap(), 7);
    assert_eq!(worker.evaluate::<i32>("1 + 2").await.unwrap(), 3);
    assert_eq!(
        worker
            .evaluate_with::<i32, _>("a => a * 3", 5)
            .await
            .unwrap(),
        15
    );
    // Worker console messages are page console events.
    let text = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(e) = events.next().await {
            if let PageEvent::Console(m) = e {
                return m.text().to_owned();
            }
        }
        String::new()
    })
    .await
    .unwrap();
    assert_eq!(text, "from worker");
    page.evaluate::<()>("() => window.w.terminate()")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), worker.wait_for_close())
        .await
        .unwrap();
    assert!(worker.is_closed());
    assert!(page.workers().is_empty());
    browser.close().await.unwrap();
}
