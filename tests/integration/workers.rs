//! WebSockets and web workers (expectations recorded from Playwright 1.63).

use std::time::Duration;

use async_tungstenite::tungstenite::Message;
use chromiumoxide::{Chromium, LaunchOptions, PageEvent, WebSocketEvent, WsPayload};
use futures::StreamExt;

use crate::server::TestServer;

/// Echoes `echo:<text>` plus a binary [1, 2, 3] for each text message.
pub(crate) async fn ws_server() -> u16 {
    let listener = crate::server::browsable_listener().await;
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
                        // Keep reading after the client's close frame:
                        // tungstenite sends the close reply (the handshake)
                        // and the stream ends once the client closes.
                        Message::Close(_) => {}
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
    // Worker console messages are page console events. The message is
    // logged after `self.x = 7`, so evaluating after it sees the value.
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
    assert_eq!(worker.evaluate::<i32>("() => self.x").await.unwrap(), 7);
    assert_eq!(worker.evaluate::<i32>("1 + 2").await.unwrap(), 3);
    assert_eq!(
        worker
            .evaluate_with::<i32, _>("a => a * 3", 5)
            .await
            .unwrap(),
        15
    );
    let handle = worker.evaluate_handle("() => ({a: 1})").await.unwrap();
    assert_eq!(
        handle.json_value::<serde_json::Value>().await.unwrap(),
        serde_json::json!({"a": 1})
    );
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

#[tokio::test]
async fn route_web_socket_like_playwright() {
    use chromiumoxide::WebSocketRoute;
    use std::sync::{Arc, Mutex};
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>x</p>", 0);
    let port = ws_server().await;
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    let l = log.clone();
    page.route_web_socket("**/mock", move |ws: WebSocketRoute| {
        let l = l.clone();
        async move {
            l.lock().unwrap().push(format!(
                "route {}",
                ws.url().split(':').next_back().unwrap_or("")
            ));
            let (l2, w2) = (l.clone(), ws.clone());
            ws.on_message(move |m: WsPayload| {
                let (l2, w2) = (l2.clone(), w2.clone());
                async move {
                    if let WsPayload::Text(t) = m {
                        l2.lock().unwrap().push(format!("page sent {t}"));
                        w2.send(format!("reply:{t}")).await;
                    }
                }
            });
            let l3 = l.clone();
            ws.on_close(move |code: Option<u16>, reason: String| {
                let l3 = l3.clone();
                async move {
                    l3.lock()
                        .unwrap()
                        .push(format!("page closed {} {reason}", code.unwrap_or(0)));
                }
            });
        }
    })
    .await
    .unwrap();
    page.route_web_socket("**/proxy", |ws: WebSocketRoute| async move {
        let server = ws.connect_to_server().unwrap();
        let s2 = server.clone();
        ws.on_message(move |m: WsPayload| {
            let s2 = s2.clone();
            async move {
                if let WsPayload::Text(t) = m {
                    s2.send(t.to_uppercase()).await;
                }
            }
        });
        let w2 = ws.clone();
        server.on_message(move |m: WsPayload| {
            let w2 = w2.clone();
            async move {
                if let WsPayload::Text(t) = m {
                    w2.send(format!("{t}!")).await;
                }
            }
        });
    })
    .await
    .unwrap();
    page.goto(&server.url("/")).await.unwrap();
    let open = |path: &str, send: &str, close: bool| {
        format!(
            "() => new Promise(res => {{ const ws = new WebSocket('ws://127.0.0.1:{port}/{path}'); \
             ws.onopen = () => ws.send('{send}'); ws.onmessage = e => {{ {} res(e.data); }}; }})",
            if close { "ws.close(4000, 'bye');" } else { "" }
        )
    };
    let got: String = page.evaluate(&open("mock", "hi", true)).await.unwrap();
    assert_eq!(got, "reply:hi");
    let got: String = page.evaluate(&open("proxy", "hello", false)).await.unwrap();
    assert_eq!(got, "echo:HELLO!");
    let got: String = page.evaluate(&open("other", "raw", false)).await.unwrap();
    assert_eq!(got, "echo:raw");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let log = log.lock().unwrap().clone();
    assert_eq!(
        log,
        [
            format!("route {port}/mock"),
            "page sent hi".into(),
            "page closed 4000 bye".into()
        ]
    );
    browser.close().await.unwrap();
}
