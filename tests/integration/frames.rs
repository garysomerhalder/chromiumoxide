use chromiumoxide::{Chromium, LaunchOptions};

const NESTED: &str = r#"<title>top</title>
<iframe name=outer style="position:absolute;left:40px;top:60px;width:400px;height:300px;border:5px solid black"
 srcdoc="<title>outer</title><p id=o>outer text</p>
 <iframe name=inner style='margin-left:30px;width:300px;height:150px;border:0' srcdoc='<title>inner</title><button onclick=&quot;parent.parent.innerClicked = true&quot; style=&quot;margin:20px&quot;>Inner button</button>'></iframe>"></iframe>"#;

#[tokio::test]
async fn frames_tree_and_frame_evaluation() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(NESTED).await.unwrap();
    page.wait_for_function("() => window.frames.length === 1 && window.frames[0].frames.length === 1 && window.frames[0].frames[0].document.readyState === 'complete'")
        .await
        .unwrap();
    let frames = page.frames();
    assert_eq!(frames.len(), 3);
    let main = page.main_frame();
    assert!(main.parent_frame().is_none());
    let outer = page.frame("outer").expect("outer frame");
    assert_eq!(outer.parent_frame().unwrap().id(), main.id());
    assert_eq!(main.child_frames().len(), 1);
    let inner = outer.child_frames().pop().unwrap();
    assert_eq!(inner.name(), "inner");
    assert_eq!(
        inner.evaluate::<String>("document.title").await.unwrap(),
        "inner"
    );
    assert_eq!(outer.title().await.unwrap(), "outer");
    assert_eq!(
        outer.locator("#o").inner_text().await.unwrap(),
        "outer text"
    );
    assert!(outer.content().await.unwrap().contains("outer text"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn frame_locators_click_inside_nested_iframes() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(NESTED).await.unwrap();
    let button = page
        .frame_locator("iframe")
        .frame_locator("iframe")
        .get_by_role(
            "button",
            chromiumoxide::RoleOptions {
                name: Some("Inner button".into()),
                ..Default::default()
            },
        );
    button.click().await.unwrap();
    assert!(
        page.evaluate::<bool>("window.innerClicked === true")
            .await
            .unwrap()
    );
    let text = page.locator("iframe").content_frame().locator("#o");
    assert_eq!(text.inner_text().await.unwrap(), "outer text");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn frame_evaluate_runs_statement_strings() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content("<iframe name=f srcdoc='<title>child</title>'></iframe>")
        .await
        .unwrap();
    page.wait_for_function("() => window.frames[0] && window.frames[0].document.title === 'child'")
        .await
        .unwrap();
    let frame = page.frame("f").unwrap();
    assert_eq!(
        frame.evaluate::<i64>("const a = 1; a + 1").await.unwrap(),
        2
    );
    assert_eq!(
        frame
            .evaluate::<String>("let t = document.title; t")
            .await
            .unwrap(),
        "child"
    );
    assert_eq!(frame.evaluate::<i64>("() => 3").await.unwrap(), 3);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn child_frame_wait_for_navigation() {
    let server = crate::server::TestServer::start().await;
    server.route("/a.html", "text/html", "<p>a</p>", 0);
    server.route("/b.html", "text/html", "<p>b</p>", 0);
    server.route(
        "/top.html",
        "text/html",
        "<iframe name=f src='/a.html'></iframe>",
        0,
    );
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/top.html")).await.unwrap();
    let frame = page.frame("f").expect("child frame");

    let response = frame
        .wait_for_navigation()
        .run(frame.evaluate::<()>("() => { location.href = '/b.html'; }"))
        .await
        .unwrap()
        .expect("a response");
    assert_eq!(response.url, server.url("/b.html"));
    assert!(frame.url().ends_with("/b.html"));

    // Same-document navigations resolve without a response.
    let r = frame
        .wait_for_navigation()
        .run(frame.evaluate::<()>("() => { location.hash = 'x'; }"))
        .await
        .unwrap();
    assert!(r.is_none());

    // The page's (main frame's) wait ignores child navigations.
    let err = page
        .wait_for_navigation()
        .timeout(std::time::Duration::from_millis(500))
        .run(frame.evaluate::<()>("() => { location.href = '/a.html'; }"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("waiting for navigation"), "{err}");
    browser.close().await.unwrap();
}
