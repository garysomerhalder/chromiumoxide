//! WebKit (Playwright's WebKit build) against the same Playwright API the
//! Firefox engine passes: the Firefox tests, ported.

use chromiumoxide::WebKit;

fn options() -> Option<chromiumoxide::webkit::WebKitLaunchOptions> {
    crate::webkit::options()
}

/// WebKit: the rest of Playwright's Locator (states, blur, clear,
/// dispatchEvent, evaluateAll, selectText, setChecked, scrollIntoView,
/// describe, normalize, visible, within, contentFrame, drop, tap).
#[tokio::test]
async fn webkit_locator_helpers() {
    use chromiumoxide::DropPayload;
    let Some(options) = options() else {
        return;
    };
    let browser = WebKit::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<input id=t value=hello><input id=ro readonly value=r><button id=off disabled>off</button>\
         <input id=c type=checkbox><p id=p>para</p><ul><li>a</li><li>b</li></ul>\
         <img alt='Logo' title='Tip' src='data:,'><label>Name <input id=n placeholder=Your></label>\
         <div style='height:3000px'></div><div id=far>far</div>\
         <div id=drop style='width:100px;height:50px'>drop</div>\
         <iframe srcdoc='<b>inner</b>'></iframe>\
         <script>window.events = [];\
         document.getElementById('t').addEventListener('blur', () => events.push('blur'));\
         document.getElementById('p').addEventListener('custom', e => events.push(e.type));\
         const d = document.getElementById('drop');\
         d.addEventListener('dragover', e => e.preventDefault());\
         d.addEventListener('drop', e => { e.preventDefault(); events.push('drop:' + e.dataTransfer.getData('text/plain')); });\
         </script>",
    )
    .await
    .unwrap();

    // States.
    assert!(page.locator("#t").is_enabled().await.unwrap());
    assert!(page.locator("#off").is_disabled().await.unwrap());
    assert!(page.locator("#t").is_editable().await.unwrap());
    assert!(!page.locator("#ro").is_editable().await.unwrap());

    // Focus, blur, clear, selectText.
    page.locator("#t").focus().await.unwrap();
    page.locator("#t").blur().await.unwrap();
    page.locator("#t").select_text().await.unwrap();
    assert_eq!(
        page.evaluate::<String>("String(getSelection()) || (document.activeElement.value.substring(document.activeElement.selectionStart, document.activeElement.selectionEnd))")
            .await
            .unwrap(),
        "hello"
    );
    page.locator("#t").clear().await.unwrap();
    assert_eq!(page.locator("#t").input_value().await.unwrap(), "");

    // setChecked, dispatchEvent, evaluateAll.
    page.locator("#c").set_checked(true).await.unwrap();
    assert!(page.locator("#c").is_checked().await.unwrap());
    page.locator("#c").set_checked(false).await.unwrap();
    assert!(!page.locator("#c").is_checked().await.unwrap());
    page.locator("#p").dispatch_event("custom").await.unwrap();
    assert_eq!(
        page.locator("li")
            .evaluate_all::<Vec<String>>("els => els.map(e => e.textContent)")
            .await
            .unwrap(),
        ["a", "b"]
    );

    // Scrolling.
    page.locator("#far")
        .scroll_into_view_if_needed()
        .await
        .unwrap();
    assert!(page.evaluate::<f64>("scrollY").await.unwrap() > 1000.0);

    // get_by_* inside a locator, describe, visible, within, normalize.
    let body = page.locator("body");
    assert_eq!(body.get_by_alt_text("Logo", true).count().await.unwrap(), 1);
    assert_eq!(body.get_by_title("Tip", true).count().await.unwrap(), 1);
    assert_eq!(body.get_by_label("Name", false).count().await.unwrap(), 1);
    assert_eq!(
        body.get_by_placeholder("Your", true).count().await.unwrap(),
        1
    );
    let described = page.locator("#p").describe("the paragraph");
    assert_eq!(described.description().as_deref(), Some("the paragraph"));
    assert_eq!(described.inner_text().await.unwrap(), "para");
    assert_eq!(page.locator("li").visible(true).count().await.unwrap(), 2);
    assert_eq!(
        page.locator("li")
            .within(&page.locator("ul"))
            .count()
            .await
            .unwrap(),
        2
    );
    let normal = page.get_by_text("para", true).normalize().await.unwrap();
    assert_eq!(normal.inner_text().await.unwrap(), "para");
    assert!(!format!("{}", page.locator("#p")).is_empty());
    page.locator("#p")
        .wait_for_function("e => e.textContent === 'para'")
        .await
        .unwrap();

    // contentFrame and frameLocator from a locator.
    assert_eq!(
        page.locator("iframe")
            .content_frame()
            .locator("b")
            .inner_text()
            .await
            .unwrap(),
        "inner"
    );
    assert_eq!(
        page.locator("body")
            .frame_locator("iframe")
            .locator("b")
            .count()
            .await
            .unwrap(),
        1
    );

    // Highlight (no visible result to check beyond not failing).
    page.locator("#p").highlight().await.unwrap();
    page.locator("#p").hide_highlight().await.unwrap();

    // External drop.
    page.locator("#drop")
        .drop(DropPayload::default().data("text/plain", "dropped"))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<Vec<String>>("events").await.unwrap(),
        // `blur()`, then the checkbox click takes the focus `clear()` gave back.
        ["blur", "blur", "custom", "drop:dropped"]
    );

    // tap needs a touch context, which WebKit needs hasTouch for.
    let err = page.locator("#p").tap().await.unwrap_err().to_string();
    assert!(err.contains("hasTouch"), "{err}");
    browser.close().await.unwrap();
}

/// WebKit: Frame objects (main frame, child frames, parent, name, goto,
/// content, locators, waits, script and style tags) and Playwright's
/// selector shortcuts on Page and Frame (`page.click(selector)`, ...).
#[tokio::test]
async fn webkit_frames_and_selector_shortcuts() {
    use chromiumoxide::{ScriptTag, StyleTag, WaitUntil as LifecycleEvent};
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route(
        "/main.html",
        "text/html",
        "<title>main</title><iframe name=kid src='/kid.html'></iframe>",
        0,
    );
    server.route(
        "/kid.html",
        "text/html",
        "<title>kid</title><input id=i><button id=b onclick=\"this.textContent='hit'\">b</button>\
         <input id=c type=checkbox><select id=s><option>x</option><option>y</option></select>",
        0,
    );
    server.route(
        "/other.html",
        "text/html",
        "<title>other</title><p>o</p>",
        0,
    );
    let browser = WebKit::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/main.html")).await.unwrap();

    let main = page.main_frame();
    assert!(main.parent_frame().is_none());
    assert_eq!(main.title().await.unwrap(), "main");
    let kids = main.child_frames().await.unwrap();
    assert_eq!(kids.len(), 1);
    let kid = page.frame("kid").await.unwrap().expect("frame by name");
    assert_eq!(kid.name().await.unwrap(), "kid");
    assert!(kid.url().ends_with("/kid.html"));
    assert_eq!(kid.parent_frame().unwrap().url(), main.url());
    assert!(!kid.is_detached().await.unwrap());
    assert!(std::ptr::eq(kid.page(), kid.page()));

    // Shortcuts on a frame, and its locators.
    kid.fill("#i", "typed").await.unwrap();
    assert_eq!(kid.input_value("#i").await.unwrap(), "typed");
    kid.click("#b").await.unwrap();
    assert_eq!(kid.inner_text("#b").await.unwrap(), "hit");
    assert_eq!(
        kid.text_content("#b").await.unwrap().as_deref(),
        Some("hit")
    );
    assert_eq!(kid.inner_html("#b").await.unwrap(), "hit");
    assert_eq!(
        kid.get_attribute("#i", "id").await.unwrap().as_deref(),
        Some("i")
    );
    kid.check("#c").await.unwrap();
    assert!(kid.is_checked("#c").await.unwrap());
    kid.uncheck("#c").await.unwrap();
    kid.set_checked("#c", true).await.unwrap();
    assert_eq!(kid.select_option("#s", &["y"]).await.unwrap(), ["y"]);
    assert!(kid.is_visible("#b").await.unwrap());
    assert!(kid.is_hidden("#nope").await.unwrap());
    assert!(kid.is_enabled("#b").await.unwrap());
    assert!(!kid.is_disabled("#b").await.unwrap());
    assert!(kid.is_editable("#i").await.unwrap());
    kid.focus("#i").await.unwrap();
    kid.press("#i", "End").await.unwrap();
    kid.hover("#b").await.unwrap();
    kid.dblclick("#b").await.unwrap();
    kid.dispatch_event("#b", "click").await.unwrap();
    assert_eq!(kid.locator("#b").count().await.unwrap(), 1);
    assert_eq!(
        kid.get_by_role("button", Default::default())
            .count()
            .await
            .unwrap(),
        1
    );
    assert_eq!(kid.get_by_text("hit", true).count().await.unwrap(), 1);
    assert_eq!(
        kid.evaluate::<String>("document.title").await.unwrap(),
        "kid"
    );
    assert!(kid.content().await.unwrap().contains("<title>kid</title>"));
    kid.add_style_tag(StyleTag::Content("#b { color: rgb(1, 2, 3) }".into()))
        .await
        .unwrap();
    kid.add_script_tag(ScriptTag::content("window.added = 7;"))
        .await
        .unwrap();
    assert_eq!(kid.evaluate::<i64>("window.added").await.unwrap(), 7);
    kid.wait_for_function("window.added === 7").await.unwrap();
    kid.wait_for_timeout(std::time::Duration::from_millis(1))
        .await;

    // Navigation inside the frame.
    kid.goto(&server.url("/other.html")).await.unwrap();
    kid.wait_for_url("**/other.html").await.unwrap();
    kid.wait_for_load_state(LifecycleEvent::Load).await.unwrap();
    assert_eq!(kid.title().await.unwrap(), "other");
    kid.set_content("<p id=x>set</p>").await.unwrap();
    assert_eq!(kid.inner_text("#x").await.unwrap(), "set");
    assert_eq!(
        kid.frame_locator("nope")
            .locator("p")
            .count()
            .await
            .unwrap(),
        0
    );

    // Page shortcuts.
    page.set_content("<input id=t><button onclick=\"this.textContent='p'\">b</button>")
        .await
        .unwrap();
    page.fill("#t", "x").await.unwrap();
    page.click("button").await.unwrap();
    assert_eq!(page.inner_text("button").await.unwrap(), "p");
    assert_eq!(page.input_value("#t").await.unwrap(), "x");
    page.add_script_tag(ScriptTag::content("window.p = 1;"))
        .await
        .unwrap();
    assert_eq!(page.evaluate::<i64>("window.p").await.unwrap(), 1);

    // A frame removed from the page is detached.
    page.set_content("<iframe name=gone srcdoc='x'></iframe>")
        .await
        .unwrap();
    let gone = page.frame("gone").await.unwrap().unwrap();
    page.evaluate::<()>("document.querySelector('iframe').remove()")
        .await
        .unwrap();
    assert!(gone.is_detached().await.unwrap());
    browser.close().await.unwrap();
}
