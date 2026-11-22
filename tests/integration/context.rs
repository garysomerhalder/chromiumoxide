use chromiumoxide::{
    Chromium, ColorScheme, ContextOptions, HttpCredentials, LaunchOptions, Viewport,
};
use serde_json::Value;

use crate::server::TestServer;

#[tokio::test]
async fn context_emulation_reaches_the_page() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(390, 844)),
            device_scale_factor: Some(3.0),
            has_touch: true,
            user_agent: Some("Mozilla/5.0 (Linux; Android 14; Pixel 7) Test".into()),
            locale: Some("de-DE".into()),
            timezone_id: Some("Asia/Tokyo".into()),
            color_scheme: Some(ColorScheme::Dark),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    // userAgentData only exists in secure contexts; localhost is one.
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>hi</p>", 0);
    page.goto(&server.url("/")).await.unwrap();
    let got: Value = page
        .evaluate(
            "() => ({ w: innerWidth, h: innerHeight, dpr: devicePixelRatio, sw: screen.width, ua: navigator.userAgent, \
             lang: navigator.language, tz: Intl.DateTimeFormat().resolvedOptions().timeZone, \
             dark: matchMedia('(prefers-color-scheme: dark)').matches, touch: navigator.maxTouchPoints > 0, \
             platform: navigator.userAgentData && navigator.userAgentData.platform })",
        )
        .await
        .unwrap();
    assert_eq!(got["w"], 390);
    assert_eq!(got["h"], 844);
    assert_eq!(got["dpr"], 3);
    assert_eq!(got["sw"], 390);
    assert_eq!(got["ua"], "Mozilla/5.0 (Linux; Android 14; Pixel 7) Test");
    assert_eq!(got["lang"], "de-DE");
    assert_eq!(got["tz"], "Asia/Tokyo");
    assert_eq!(got["dark"], true);
    assert_eq!(got["touch"], true);
    assert_eq!(got["platform"], "Android");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn default_context_is_1280_by_720_light() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let got: Value = page
        .evaluate("() => [innerWidth, innerHeight, devicePixelRatio, matchMedia('(prefers-color-scheme: light)').matches]")
        .await
        .unwrap();
    assert_eq!(got, serde_json::json!([1280, 720, 1, true]));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn contexts_are_isolated_and_closable() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let a = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(300, 200)),
            ..Default::default()
        })
        .await
        .unwrap();
    let b = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(600, 400)),
            ..Default::default()
        })
        .await
        .unwrap();
    let (pa, pb) = (a.new_page().await.unwrap(), b.new_page().await.unwrap());
    assert_eq!(pa.evaluate::<u32>("innerWidth").await.unwrap(), 300);
    assert_eq!(pb.evaluate::<u32>("innerWidth").await.unwrap(), 600);
    assert_eq!(browser.contexts().len(), 2);
    assert_eq!(a.pages().len(), 1);
    a.close().await.unwrap();
    assert!(pa.is_closed());
    assert!(pa.evaluate::<u32>("1").await.is_err());
    assert_eq!(pb.evaluate::<u32>("innerWidth").await.unwrap(), 600);
    assert_eq!(browser.contexts().len(), 1);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn evaluate_returns_values_and_reports_exceptions() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    assert_eq!(page.evaluate::<i64>("1 + 2").await.unwrap(), 3);
    assert_eq!(page.evaluate::<String>("() => 'hi'").await.unwrap(), "hi");
    assert_eq!(
        page.evaluate::<i64>("async () => { await new Promise(r => setTimeout(r, 10)); return 7 }")
            .await
            .unwrap(),
        7
    );
    assert_eq!(
        page.evaluate_with::<i64, _>("(x) => x.a * 2", serde_json::json!({"a": 21}))
            .await
            .unwrap(),
        42
    );
    assert_eq!(
        page.evaluate::<Option<i64>>("undefined").await.unwrap(),
        None
    );
    let err = page
        .evaluate::<Value>("() => { throw new Error('boom') }")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("boom"), "{err}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn http_credentials_answer_basic_auth() {
    let server = TestServer::start().await;
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions {
            http_credentials: Some(HttpCredentials {
                username: "user".into(),
                password: "pass".into(),
                origin: None,
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = ctx.new_page().await.unwrap();
    let response = page.goto(&server.url("/protected")).await.unwrap().unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(
        page.evaluate::<String>("document.body.textContent")
            .await
            .unwrap(),
        "secret ok"
    );

    let plain = browser.new_page().await.unwrap();
    let response = plain
        .goto(&server.url("/protected"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status, 401);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn evaluate_runs_statement_strings_like_playwright() {
    // Playwright evaluates a non-function string as a script: statements are
    // allowed, the completion value is returned and a promise is awaited.
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    assert_eq!(page.evaluate::<i64>("const a = 1; a + 1").await.unwrap(), 2);
    page.evaluate::<Value>("document.body.style.margin = 0; window.x = 1")
        .await
        .unwrap();
    assert_eq!(page.evaluate::<i64>("window.x").await.unwrap(), 1);
    assert_eq!(
        page.evaluate::<String>("let s = 'a'; s += 'b'; Promise.resolve(s + 'c')")
            .await
            .unwrap(),
        "abc"
    );
    assert_eq!(
        page.evaluate::<i64>("var f = () => 5; f()").await.unwrap(),
        5
    );
    // expressions and functions keep working
    assert_eq!(page.evaluate::<i64>("1 + 2").await.unwrap(), 3);
    assert_eq!(page.evaluate::<i64>("() => 7").await.unwrap(), 7);
    let err = page
        .evaluate::<Value>("const b = 1; throw new Error('stmt boom')")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("stmt boom"), "{err}");
    let v = page
        .wait_for_function("const t = window.x; t === 1")
        .await
        .unwrap();
    assert_eq!(v, true);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn service_workers_can_be_blocked() {
    use chromiumoxide::ServiceWorkers;
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>sw</p>", 0);
    server.route("/sw.js", "text/javascript", "self.x = 1", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            service_workers: ServiceWorkers::Block,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    let warning = page
        .wait_for_console_message(|m| m.kind() == "warning")
        .run(page.evaluate::<bool>(
            "async () => (await navigator.serviceWorker.register('/sw.js')) === undefined",
        ))
        .await
        .unwrap();
    assert_eq!(
        warning.text(),
        "Service Worker registration blocked by chromiumoxide"
    );
    assert!(
        page.evaluate::<bool>(
            "async () => (await navigator.serviceWorker.register('/sw.js')) === undefined"
        )
        .await
        .unwrap()
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn http_credentials_can_change_at_runtime() {
    let server = TestServer::start().await;
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    let r = page.goto(&server.url("/protected")).await.unwrap().unwrap();
    assert_eq!(r.status, 401);
    context
        .set_http_credentials(Some(HttpCredentials {
            username: "user".into(),
            password: "pass".into(),
            origin: None,
        }))
        .await
        .unwrap();
    let r = page.goto(&server.url("/protected")).await.unwrap().unwrap();
    assert_eq!(r.status, 200);
    // Like Playwright, clearing them later keeps working for this origin:
    // Chromium has cached the credentials (Playwright 1.63 also gets 200).
    context.set_http_credentials(None).await.unwrap();
    let r = page.goto(&server.url("/protected")).await.unwrap().unwrap();
    assert_eq!(r.status, 200);
    browser.close().await.unwrap();
}

/// A tiny forward proxy: answers 407 until `Proxy-Authorization` carries
/// user:pass, then serves every URL itself; `/site-auth` asks the *site*
/// credentials (401) and records any `Authorization` header it gets.
struct TestProxy {
    port: u16,
    site_authorizations: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

async fn test_proxy() -> TestProxy {
    use base64::Engine;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let site_authorizations = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("user:pass")
    );
    let (sa, rq) = (site_authorizations.clone(), requests.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let (sa, rq, expected) = (sa.clone(), rq.clone(), expected.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                let url = head
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                let header = |name: &str| {
                    head.lines()
                        .find(|l| l.to_ascii_lowercase().starts_with(&format!("{name}:")))
                        .map(|l| l[name.len() + 1..].trim().to_owned())
                };
                rq.lock().unwrap().push(url.clone());
                let reply = if header("proxy-authorization").as_deref() != Some(expected.as_str()) {
                    "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"proxy\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                } else if url.ends_with("/site-auth") {
                    if let Some(a) = header("authorization") {
                        sa.lock().unwrap().push(a);
                    }
                    "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"site\"\r\nContent-Type: text/html\r\nContent-Length: 9\r\nConnection: close\r\n\r\nsite auth".to_owned()
                } else {
                    let body = format!("<p>proxied {url}</p>");
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    TestProxy {
        port,
        site_authorizations,
        requests,
    }
}

/// `proxy` on launch and on contexts; Playwright 1.65: the proxy's
/// credentials answer only proxy challenges, never a site's.
#[tokio::test]
async fn proxy_with_credentials_for_the_proxy_only() {
    let proxy = test_proxy().await;
    let browser = Chromium::launch(LaunchOptions {
        proxy: Some(chromiumoxide::Proxy {
            server: format!("http://127.0.0.1:{}", proxy.port),
            username: Some("user".into()),
            password: Some("pass".into()),
            ..Default::default()
        }),
        ..Default::default()
    })
    .await
    .unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto("http://site.test/ok").await.unwrap();
    assert_eq!(
        page.locator("p").inner_text().await.unwrap(),
        "proxied http://site.test/ok"
    );
    let r = page
        .goto("http://site.test/site-auth")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status(), 401);
    assert!(
        proxy.site_authorizations.lock().unwrap().is_empty(),
        "proxy credentials were sent to the site"
    );
    browser.close().await.unwrap();

    // A context's own proxy.
    let other = test_proxy().await;
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            proxy: Some(chromiumoxide::Proxy {
                server: format!("http://127.0.0.1:{}", other.port),
                username: Some("user".into()),
                password: Some("pass".into()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto("http://ctx.test/hello").await.unwrap();
    assert_eq!(
        page.locator("p").inner_text().await.unwrap(),
        "proxied http://ctx.test/hello"
    );
    assert!(
        other
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|u| u == "http://ctx.test/hello")
    );
    browser.close().await.unwrap();
}
