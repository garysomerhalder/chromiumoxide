//! baseURL, script/CSP switches, selectors, aria JSON, persistent contexts and
//! remaining network members (expectations recorded from Playwright 1.63).

use chromiumoxide::{Chromium, ContextOptions, LaunchOptions, ScriptTag, Viewport};
use serde_json::{Value, json};

use crate::server::TestServer;

#[tokio::test]
async fn base_url_and_context_switches() {
    let server = TestServer::start().await;
    server.route("/app/sub/page", "text/html", "<p>sub</p>", 0);
    server.route("/root", "text/html", "<p>root</p>", 0);
    server.respond(
        "/csp",
        "200 OK",
        &[
            ("Content-Type", "text/html"),
            ("Content-Security-Policy", "script-src 'none'"),
        ],
        "<p>csp</p>",
    );
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let c = browser
        .new_context(ContextOptions {
            base_url: Some(server.url("/app/")),
            ..Default::default()
        })
        .await
        .unwrap();
    let p = c.new_page().await.unwrap();
    p.goto("sub/page").await.unwrap();
    assert_eq!(p.url(), server.url("/app/sub/page"));
    p.goto("/root").await.unwrap();
    assert_eq!(p.url(), server.url("/root"));
    p.wait_for_url("**/root").await.unwrap();

    let c2 = browser
        .new_context(ContextOptions {
            java_script_enabled: false,
            ..Default::default()
        })
        .await
        .unwrap();
    let p2 = c2.new_page().await.unwrap();
    p2.set_content("<script>document.title='js'</script><noscript>nojs</noscript>")
        .await
        .unwrap();
    assert_eq!(p2.title().await.unwrap(), "");

    let c3 = browser
        .new_context(ContextOptions {
            bypass_csp: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let p3 = c3.new_page().await.unwrap();
    p3.goto(&server.url("/csp")).await.unwrap();
    p3.add_script_tag(ScriptTag::content("window.x = 1"))
        .await
        .unwrap();
    assert_eq!(p3.evaluate::<i32>("() => window.x").await.unwrap(), 1);
    let p4 = browser.new_page().await.unwrap();
    p4.goto(&server.url("/csp")).await.unwrap();
    assert!(
        p4.add_script_tag(ScriptTag::content("window.x = 1"))
            .await
            .is_err()
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn selectors_and_aria_json() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    browser.selectors().set_test_id_attribute("data-qa");
    browser
        .selectors()
        .register(
            "tag",
            "({ query(root, s) { return root.querySelector(s); }, queryAll(root, s) { return Array.from(root.querySelectorAll(s)); } })",
        )
        .unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content("<b data-qa=x>Q</b><b data-testid=x>T</b>")
        .await
        .unwrap();
    assert_eq!(
        page.get_by_test_id("x")
            .text_content()
            .await
            .unwrap()
            .as_deref(),
        Some("Q")
    );
    assert_eq!(page.locator("tag=b").count().await.unwrap(), 2);
    assert_eq!(
        page.locator("body").aria_snapshot_json().await.unwrap(),
        json!([{"role": "text", "text": "QT"}])
    );
    assert_eq!(
        page.aria_snapshot_json().await.unwrap(),
        json!([{"role": "text", "text": "QT"}])
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn persistent_context_keeps_the_profile() {
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>p</p>", 0);
    let dir = std::env::temp_dir().join(format!("chromiumoxide-persist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let options = || ContextOptions {
        viewport: Some(Viewport::new(400, 300)),
        ..Default::default()
    };
    let context = Chromium::launch_persistent_context(&dir, LaunchOptions::default(), options())
        .await
        .unwrap();
    let page = match context.pages().into_iter().next() {
        Some(p) => p,
        None => context.wait_for_page().await.unwrap(),
    };
    page.goto(&server.url("/")).await.unwrap();
    assert_eq!(page.evaluate::<i32>("() => innerWidth").await.unwrap(), 400);
    page.evaluate::<()>("() => localStorage.setItem('kept', 'yes')")
        .await
        .unwrap();
    let extra = context.new_page().await.unwrap();
    assert_eq!(context.pages().len(), 2);
    extra.close().await.unwrap();
    context.close().await.unwrap();
    assert!(dir.exists(), "the profile stays");

    let again = Chromium::launch_persistent_context(&dir, LaunchOptions::default(), options())
        .await
        .unwrap();
    let page = again.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    assert_eq!(
        page.evaluate::<Option<String>>("() => localStorage.getItem('kept')")
            .await
            .unwrap()
            .as_deref(),
        Some("yes")
    );
    again.close().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn request_header_helpers_and_post_json() {
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>x</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    let resp = page
        .wait_for_response("**/echo")
        .run(page.evaluate::<u16>(
            "() => fetch('/echo', {method: 'POST', body: JSON.stringify({a: 1}), headers: {'Content-Type': 'application/json', 'X-Two': 'v'}}).then(r => r.status)",
        ))
        .await
        .unwrap();
    let req = resp.request();
    assert_eq!(
        req.post_data_json::<Value>().unwrap(),
        Some(json!({"a": 1}))
    );
    assert_eq!(
        req.header_value("x-two").await.unwrap().as_deref(),
        Some("v")
    );
    assert!(
        req.headers_array()
            .await
            .unwrap()
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("x-two") && h.value == "v")
    );
    assert!(
        resp.headers_array()
            .await
            .unwrap()
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("content-type"))
    );
    assert!(resp.security_details().is_none(), "plain HTTP");
    let form = page
        .wait_for_request("**/echo")
        .run(page.evaluate::<u16>(
            "() => fetch('/echo', {method: 'POST', body: 'a=1&b=two+words', headers: {'Content-Type': 'application/x-www-form-urlencoded'}}).then(r => r.status)",
        ))
        .await
        .unwrap();
    assert_eq!(
        form.post_data_json::<Value>().unwrap(),
        Some(json!({"a": "1", "b": "two words"}))
    );
    // Dialog::page.
    page.on_dialog(|d| async move {
        let _ = d.dismiss().await;
    });
    let dialog = page
        .wait_for_event(|e| match e {
            chromiumoxide::PageEvent::Dialog(d) => Some(d.clone()),
            _ => None,
        })
        .run(page.evaluate::<()>("() => { setTimeout(() => alert('x')) }"))
        .await
        .unwrap();
    assert_eq!(dialog.page(), &page);
    browser.close().await.unwrap();
}
