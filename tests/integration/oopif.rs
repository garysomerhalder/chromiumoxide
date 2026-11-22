//! Out-of-process (cross-site) iframes: `127.0.0.1` embedding `localhost` is
//! cross-site, so Chromium runs the iframe in its own process and target.

use std::time::Duration;

use chromiumoxide::{Chromium, LaunchOptions, expect};

use crate::server::TestServer;

#[tokio::test]
async fn cross_site_iframes_work_like_frames() {
    let server = TestServer::start().await;
    let child = server.url("/child.html").replace("127.0.0.1", "localhost");
    server.route(
        "/child.html",
        "text/html",
        "<title>child</title><button style='margin:30px' onclick=\"this.textContent = 'clicked ' + location.host\">Press</button><input id=i>",
        0,
    );
    server.route(
        "/top.html",
        "text/html",
        format!(
            "<title>top</title><div style='height:50px'></div><iframe name=x style='margin-left:70px;width:400px;height:200px;border:3px solid red' src='{child}'></iframe>"
        ),
        0,
    );
    // Site isolation puts the cross-site iframe in its own process.
    let browser = Chromium::launch(LaunchOptions {
        args: vec!["--site-per-process".into()],
        ..Default::default()
    })
    .await
    .unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/top.html")).await.unwrap();

    let mut frame = None;
    for _ in 0..50 {
        frame = page
            .frames()
            .into_iter()
            .find(|f| f.url().starts_with("http://localhost"));
        if frame.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let frame = frame.expect("the cross-site frame is listed");
    assert_eq!(page.frames().len(), 2);
    assert_eq!(frame.name(), "x");
    assert_eq!(frame.parent_frame().unwrap().id(), page.main_frame().id());
    frame
        .wait_for_load_state(chromiumoxide::WaitUntil::Load)
        .await
        .unwrap();
    assert_eq!(frame.title().await.unwrap(), "child");
    let host: String = frame.evaluate("location.host").await.unwrap();
    assert!(host.starts_with("localhost:"), "{host}");

    // Locators, actions and assertions inside it.
    let inside = page.frame_locator("iframe").locator("button");
    inside.click().await.unwrap();
    expect(&inside)
        .to_have_text(format!("clicked {host}"))
        .await
        .unwrap();
    let input = page.frame_locator("iframe").locator("#i");
    input.fill("typed").await.unwrap();
    assert_eq!(input.input_value().await.unwrap(), "typed");
    let b = inside.bounding_box().await.unwrap().unwrap();
    // 70px margin + 3px border + 30px button margin (body margin 8px).
    assert!((b.x - (70.0 + 8.0 + 3.0 + 30.0 + 8.0)).abs() < 2.0, "{b:?}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn routes_init_scripts_and_bindings_reach_cross_site_iframes() {
    let server = TestServer::start().await;
    let child = server.url("/child.html").replace("127.0.0.1", "localhost");
    server.route("/child.html", "text/html", "<p id=r>original</p>", 0);
    server.route(
        "/top.html",
        "text/html",
        format!("<iframe name=x src='{child}'></iframe>"),
        0,
    );
    let browser = Chromium::launch(LaunchOptions {
        args: vec!["--site-per-process".into()],
        ..Default::default()
    })
    .await
    .unwrap();
    let ctx = browser.new_context(Default::default()).await.unwrap();
    ctx.route("**/child.html", |route: chromiumoxide::Route| async move {
        route
            .fulfill()
            .body("<p id=r>routed</p><script src='/sub.js'></script>")
            .content_type("text/html")
            .await
            .unwrap();
    })
    .await
    .unwrap();
    // A sub-resource requested from inside the iframe's own process.
    ctx.route("**/sub.js", |route: chromiumoxide::Route| async move {
        route
            .fulfill()
            .body("window.__sub = 'routed'")
            .content_type("text/javascript")
            .await
            .unwrap();
    })
    .await
    .unwrap();
    ctx.expose_function("hello", |args: Vec<serde_json::Value>| async move {
        Ok(serde_json::json!(format!(
            "hi {}",
            args[0].as_str().unwrap_or_default()
        )))
    })
    .await
    .unwrap();
    let page = ctx.new_page().await.unwrap();
    page.add_init_script("window.__init = 'yes'").await.unwrap();
    page.goto(&server.url("/top.html")).await.unwrap();
    let mut frame = None;
    for _ in 0..100 {
        frame = page
            .frames()
            .into_iter()
            .find(|f| f.url().starts_with("http://localhost"));
        if frame.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let frame = frame.expect("the cross-site frame");
    frame
        .wait_for_load_state(chromiumoxide::WaitUntil::Load)
        .await
        .unwrap();
    assert_eq!(
        frame.locator("#r").text_content().await.unwrap().as_deref(),
        Some("routed")
    );
    assert_eq!(
        frame.evaluate::<String>("window.__init").await.unwrap(),
        "yes"
    );
    assert_eq!(
        frame.evaluate::<String>("hello('frame')").await.unwrap(),
        "hi frame"
    );
    browser.close().await.unwrap();
}
