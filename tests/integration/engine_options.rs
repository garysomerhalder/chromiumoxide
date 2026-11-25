//! Context options Firefox and WebKit handle beyond emulation: `proxy`
//! (with credentials for the proxy only), `clientCertificates`,
//! `bypassCSP`, `serviceWorkers: "block"` and (WebKit) `javaScriptEnabled`.

use chromiumoxide::{ClientCertificate, ContextOptions, Firefox, ServiceWorkers, WebKit};

fn proxy(port: u16) -> chromiumoxide::Proxy {
    chromiumoxide::Proxy {
        server: format!("http://127.0.0.1:{port}"),
        username: Some("user".into()),
        password: Some("pass".into()),
        ..Default::default()
    }
}

/// A page whose CSP forbids inline scripts.
const CSP_PAGE: &str = "<p id=p>csp</p>";

fn csp_server(server: &crate::server::TestServer) {
    server.respond(
        "/csp.html",
        "200 OK",
        &[
            ("Content-Type", "text/html"),
            ("Content-Security-Policy", "script-src 'none'"),
        ],
        CSP_PAGE,
    );
}

/// Adds an inline script; true when it ran.
const INLINE_RUNS: &str = "() => { const s = document.createElement('script'); \
     s.textContent = 'window.ran = true'; document.head.appendChild(s); return window.ran === true; }";

#[tokio::test]
async fn firefox_proxy_certificates_csp_and_service_workers() {
    let Some(options) = crate::firefox::options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();

    // A context's own proxy; its credentials answer the proxy only.
    let p = crate::context::test_proxy().await;
    let context = browser
        .new_context(ContextOptions {
            proxy: Some(proxy(p.port)),
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
    page.goto("http://ctx.test/site-auth").await.unwrap();
    assert!(p.site_authorizations.lock().unwrap().is_empty());
    context.close().await.unwrap();

    // bypassCSP: an inline script runs despite the page's CSP header.
    let server = crate::server::TestServer::start().await;
    csp_server(&server);
    for (bypass, runs) in [(false, false), (true, true)] {
        let context = browser
            .new_context(ContextOptions {
                bypass_csp: bypass,
                ..Default::default()
            })
            .await
            .unwrap();
        let page = context.new_page().await.unwrap();
        page.goto(&server.url("/csp.html")).await.unwrap();
        assert_eq!(
            page.evaluate::<bool>(INLINE_RUNS).await.unwrap(),
            runs,
            "bypassCSP {bypass}"
        );
        context.close().await.unwrap();
    }

    // serviceWorkers: "block".
    server.route("/sw.html", "text/html", "<p>sw</p>", 0);
    server.route("/sw.js", "text/javascript", "self.x = 1;", 0);
    let context = browser
        .new_context(ContextOptions {
            service_workers: ServiceWorkers::Block,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/sw.html")).await.unwrap();
    let registered: bool = page
        .evaluate("navigator.serviceWorker.register('/sw.js').then(r => !!r)")
        .await
        .unwrap();
    assert!(!registered);
    context.close().await.unwrap();

    // clientCertificates.
    if let Some(mtls) = crate::client_certs::mtls_server() {
        let origin = format!("https://localhost:{}", mtls.port);
        let context = browser
            .new_context(ContextOptions {
                ignore_https_errors: true,
                client_certificates: vec![ClientCertificate {
                    origin: origin.clone(),
                    cert_path: Some(mtls.dir.join("client.pem")),
                    key_path: Some(mtls.dir.join("client.key")),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .await
            .unwrap();
        let page = context.new_page().await.unwrap();
        page.goto(&format!("{origin}/")).await.unwrap();
        let text = page.locator("body").inner_text().await.unwrap();
        assert!(text.contains("Subject: CN=Alice"), "{text}");
        context.close().await.unwrap();
    }
    browser.close().await.unwrap();
}

#[tokio::test]
async fn webkit_proxy_certificates_csp_service_workers_and_javascript() {
    let Some(options) = crate::webkit::options() else {
        return;
    };
    let browser = WebKit::launch(options).await.unwrap();

    let p = crate::context::test_proxy().await;
    let context = browser
        .new_context(ContextOptions {
            proxy: Some(proxy(p.port)),
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
    page.goto("http://ctx.test/site-auth").await.unwrap();
    assert!(p.site_authorizations.lock().unwrap().is_empty());
    context.close().await.unwrap();

    let server = crate::server::TestServer::start().await;
    csp_server(&server);
    for (bypass, runs) in [(false, false), (true, true)] {
        let context = browser
            .new_context(ContextOptions {
                bypass_csp: bypass,
                ..Default::default()
            })
            .await
            .unwrap();
        let page = context.new_page().await.unwrap();
        page.goto(&server.url("/csp.html")).await.unwrap();
        assert_eq!(
            page.evaluate::<bool>(INLINE_RUNS).await.unwrap(),
            runs,
            "bypassCSP {bypass}"
        );
        context.close().await.unwrap();
    }

    server.route("/sw.html", "text/html", "<p>sw</p>", 0);
    server.route("/sw.js", "text/javascript", "self.x = 1;", 0);
    let context = browser
        .new_context(ContextOptions {
            service_workers: ServiceWorkers::Block,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/sw.html")).await.unwrap();
    let registered: bool = page
        .evaluate("navigator.serviceWorker ? navigator.serviceWorker.register('/sw.js').then(r => !!r) : false")
        .await
        .unwrap();
    assert!(!registered);
    context.close().await.unwrap();

    // javaScriptEnabled: false — the page's own scripts do not run.
    server.route(
        "/js.html",
        "text/html",
        "<p id=p>off</p><script>document.getElementById('p').textContent = 'on'</script>",
        0,
    );
    let context = browser
        .new_context(ContextOptions {
            java_script_enabled: false,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/js.html")).await.unwrap();
    assert_eq!(page.locator("#p").inner_text().await.unwrap(), "off");
    context.close().await.unwrap();

    if let Some(mtls) = crate::client_certs::mtls_server() {
        let origin = format!("https://localhost:{}", mtls.port);
        let context = browser
            .new_context(ContextOptions {
                ignore_https_errors: true,
                client_certificates: vec![ClientCertificate {
                    origin: origin.clone(),
                    cert_path: Some(mtls.dir.join("client.pem")),
                    key_path: Some(mtls.dir.join("client.key")),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .await
            .unwrap();
        let page = context.new_page().await.unwrap();
        page.goto(&format!("{origin}/")).await.unwrap();
        let text = page.locator("body").inner_text().await.unwrap();
        assert!(text.contains("Subject: CN=Alice"), "{text}");
        context.close().await.unwrap();
    }
    browser.close().await.unwrap();
}
