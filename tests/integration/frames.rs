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
