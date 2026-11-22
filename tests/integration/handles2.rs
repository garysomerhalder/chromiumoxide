//! Frame actions and element handle actions (Playwright 1.63 semantics).

use chromiumoxide::{Chromium, LaunchOptions, WaitForState};
use serde_json::{Value, json};

use crate::server::TestServer;

const FORM: &str = "<input id=n value=v><input type=checkbox id=c><select id=s><option>a</option><option>b</option></select>\
<button id=b onclick=\"window.clicks=(window.clicks||0)+1\" title=\"tt\" ondblclick=\"window.dbl=1\" onmouseover=\"window.hovered=1\">Go</button>\
<input id=p placeholder=\"Find me\"><img alt=\"pic\" src=\"data:,\"><div id=d>Some <b>bold</b></div><input type=file id=f><div id=hid hidden>x</div>";

#[tokio::test]
async fn frame_actions() {
    let server = TestServer::start().await;
    server.route("/inner", "text/html", FORM, 0);
    server.route("/next", "text/html", "<p>next</p>", 0);
    server.route(
        "/outer",
        "text/html",
        "<iframe name=child src=/inner></iframe>",
        0,
    );
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/outer")).await.unwrap();
    let frame = page.frame("child").unwrap();
    frame
        .wait_for_load_state(chromiumoxide::WaitUntil::Load)
        .await
        .unwrap();
    frame.click("#b").await.unwrap();
    frame.dblclick("#b").await.unwrap();
    frame.hover("#b").await.unwrap();
    frame.fill("#n", "filled").await.unwrap();
    frame.press("#n", "End").await.unwrap();
    frame.check("#c").await.unwrap();
    assert!(frame.is_checked("#c").await.unwrap());
    frame.uncheck("#c").await.unwrap();
    frame.set_checked("#c", true).await.unwrap();
    frame.select_option("#s", &["b"]).await.unwrap();
    frame.focus("#n").await.unwrap();
    frame.dispatch_event("#b", "click").await.unwrap();
    assert_eq!(
        frame
            .evaluate::<Value>("() => [window.clicks, window.dbl, window.hovered, document.querySelector('#s').value]")
            .await
            .unwrap(),
        json!([4, 1, 1, "b"])
    );
    assert_eq!(frame.input_value("#n").await.unwrap(), "filled");
    assert_eq!(
        frame.get_attribute("#b", "title").await.unwrap().as_deref(),
        Some("tt")
    );
    assert_eq!(frame.inner_text("#d").await.unwrap(), "Some bold");
    assert_eq!(frame.inner_html("#d").await.unwrap(), "Some <b>bold</b>");
    assert_eq!(
        frame.text_content("#d").await.unwrap().as_deref(),
        Some("Some bold")
    );
    assert!(frame.is_visible("#b").await.unwrap());
    assert!(frame.is_hidden("#hid").await.unwrap());
    assert!(frame.is_enabled("#b").await.unwrap());
    assert!(!frame.is_disabled("#b").await.unwrap());
    assert!(frame.is_editable("#n").await.unwrap());
    assert_eq!(frame.get_by_title("tt", false).count().await.unwrap(), 1);
    assert_eq!(
        frame.get_by_alt_text("pic", false).count().await.unwrap(),
        1
    );
    assert_eq!(
        frame
            .get_by_placeholder("Find me", false)
            .count()
            .await
            .unwrap(),
        1
    );
    frame
        .wait_for_selector("#hid", WaitForState::Hidden)
        .await
        .unwrap();
    frame
        .wait_for_function("() => window.clicks === 4")
        .await
        .unwrap();
    let h = frame.evaluate_handle("() => document.title").await.unwrap();
    assert_eq!(h.json_value::<String>().await.unwrap(), "");
    frame
        .add_style_tag_content("#d { color: rgb(1, 2, 3) }")
        .await
        .unwrap();
    frame
        .add_script_tag_content("window.injected = 7")
        .await
        .unwrap();
    assert_eq!(
        frame
            .evaluate::<Value>(
                "() => [getComputedStyle(document.querySelector('#d')).color, window.injected]"
            )
            .await
            .unwrap(),
        json!(["rgb(1, 2, 3)", 7])
    );
    let owner = frame.frame_element().await.unwrap();
    assert_eq!(
        owner
            .evaluate::<String>("e => e.getAttribute('name')")
            .await
            .unwrap(),
        "child"
    );
    frame
        .wait_for_timeout(std::time::Duration::from_millis(10))
        .await;
    // Child-frame navigation.
    let resp = frame.goto(&server.url("/next")).await.unwrap().unwrap();
    assert_eq!(resp.status, 200);
    frame.wait_for_url("**/next").await.unwrap();
    assert_eq!(frame.url(), server.url("/next"));
    frame.set_content("<p id=x>set</p>").await.unwrap();
    assert_eq!(
        frame.text_content("#x").await.unwrap().as_deref(),
        Some("set")
    );
    // The main frame delegates to the page.
    page.main_frame().set_content(FORM).await.unwrap();
    page.main_frame().click("#b").await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn element_handle_actions() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(FORM).await.unwrap();
    let el = |s: &str| {
        let l = page.locator(s);
        async move { l.element_handle().await.unwrap() }
    };
    let b = el("#b").await;
    b.click().await.unwrap();
    b.dblclick().await.unwrap();
    b.hover().await.unwrap();
    b.dispatch_event("click").await.unwrap();
    b.scroll_into_view_if_needed().await.unwrap();
    let n = el("#n").await;
    n.fill("x").await.unwrap();
    n.press("End").await.unwrap();
    #[allow(deprecated)]
    n.type_text("yz").await.unwrap();
    assert_eq!(n.input_value().await.unwrap(), "xyz");
    n.select_text().await.unwrap();
    n.focus().await.unwrap();
    let c = el("#c").await;
    c.check().await.unwrap();
    assert!(c.is_checked().await.unwrap());
    c.uncheck().await.unwrap();
    c.set_checked(true).await.unwrap();
    el("#s").await.select_option(&["b"]).await.unwrap();
    assert_eq!(
        page.evaluate::<Value>(
            "() => [window.clicks, window.dbl, window.hovered, document.querySelector('#s').value]"
        )
        .await
        .unwrap(),
        json!([3, 1, 1, "b"])
    );
    assert_eq!(
        b.get_attribute("title").await.unwrap().as_deref(),
        Some("tt")
    );
    let d = el("#d").await;
    assert_eq!(d.inner_text().await.unwrap(), "Some bold");
    assert_eq!(d.inner_html().await.unwrap(), "Some <b>bold</b>");
    assert_eq!(
        d.text_content().await.unwrap().as_deref(),
        Some("Some bold")
    );
    assert!(b.is_visible().await.unwrap());
    assert!(el("#hid").await.is_hidden().await.unwrap());
    assert!(b.is_enabled().await.unwrap());
    assert!(!b.is_disabled().await.unwrap());
    assert!(n.is_editable().await.unwrap());
    let bb = b.bounding_box().await.unwrap().unwrap();
    assert!(bb.width > 0.0 && bb.height > 0.0);
    let png = b.screenshot().await.unwrap();
    assert_eq!(&png[..4], b"\x89PNG");
    b.wait_for_element_state("stable").await.unwrap();
    let bold = d.wait_for_selector("b").await.unwrap().unwrap();
    assert_eq!(bold.inner_text().await.unwrap(), "bold");
    let dir = std::env::temp_dir().join(format!("chromiumoxide-eh-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.txt");
    std::fs::write(&file, "hi").unwrap();
    el("#f").await.set_input_files(&[file]).await.unwrap();
    assert_eq!(
        page.evaluate::<String>("() => document.querySelector('#f').files[0].name")
            .await
            .unwrap(),
        "a.txt"
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(b.content_frame().await.unwrap().is_none());
    assert_eq!(b.owner_frame().await.unwrap().unwrap(), page.main_frame());
    browser.close().await.unwrap();
}
