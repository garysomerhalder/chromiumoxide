//! Web-first assertions (each one checked to pass in Playwright 1.63 on the
//! same page; failure messages recorded from Playwright).

use std::time::Duration;

use chromiumoxide::{Chromium, LaunchOptions, TextMatch, expect};

const PAGE: &str = r#"<title>My Page</title><div id=a class="x y" style="color: rgb(255, 0, 0)">Hello  world</div>
 <input id=i value=v aria-label="Name"><button disabled>B</button><input type=checkbox id=c checked>
 <ul><li>One</li><li>Two</li></ul><div id=e></div><div id=h hidden>h</div>
 <select id=s multiple><option value=a selected>A</option><option value=b selected>B</option></select>
 <div id=later></div><script>setTimeout(() => document.querySelector('#later').textContent = 'ready', 300)</script>"#;

fn re(p: &str, flags: &str) -> TextMatch {
    TextMatch::Regex {
        pattern: p.into(),
        flags: flags.into(),
    }
}

#[tokio::test]
async fn passing_assertions() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(PAGE).await.unwrap();
    let l = |s: &str| page.locator(s);
    expect(&l("#a")).to_be_visible().await.unwrap();
    expect(&l("#nope")).to_be_hidden().await.unwrap();
    expect(&l("#h")).to_be_hidden().await.unwrap();
    expect(&l("#h")).not().to_be_visible().await.unwrap();
    expect(&l("#a")).to_have_text("Hello world").await.unwrap();
    expect(&l("#a"))
        .to_have_text(re("hello", "i"))
        .await
        .unwrap();
    expect(&l("#a")).to_contain_text("lo wor").await.unwrap();
    expect(&l("li"))
        .to_have_texts(["One", "Two"])
        .await
        .unwrap();
    expect(&l("#later")).to_have_text("ready").await.unwrap();
    expect(&l("li")).to_have_count(2).await.unwrap();
    expect(&l("#i"))
        .to_have_attribute("value", "v")
        .await
        .unwrap();
    expect(&l("#i"))
        .to_have_attribute_present("aria-label")
        .await
        .unwrap();
    expect(&l("#a")).to_have_class("x y").await.unwrap();
    expect(&l("#a")).to_contain_class("y").await.unwrap();
    expect(&l("#a"))
        .to_have_css("color", "rgb(255, 0, 0)")
        .await
        .unwrap();
    expect(&l("#a")).to_have_id("a").await.unwrap();
    expect(&l("#i")).to_have_value("v").await.unwrap();
    expect(&l("#s")).to_have_values(["a", "b"]).await.unwrap();
    expect(&l("button")).to_be_disabled().await.unwrap();
    expect(&l("#i")).to_be_enabled().await.unwrap();
    expect(&l("#i")).to_be_editable().await.unwrap();
    expect(&l("#c")).to_be_checked().await.unwrap();
    expect(&l("#e")).to_be_empty().await.unwrap();
    expect(&l("#e")).to_be_attached().await.unwrap();
    expect(&l("button")).to_have_role("button").await.unwrap();
    expect(&l("#i"))
        .to_have_accessible_name("Name")
        .await
        .unwrap();
    expect(&l("#a")).to_be_in_viewport().await.unwrap();
    expect(&l("#c"))
        .to_have_js_property("checked", true)
        .await
        .unwrap();
    expect(&page).to_have_title("My Page").await.unwrap();
    expect(&page).to_have_url("about:blank").await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn failures_read_like_playwright() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(PAGE).await.unwrap();
    let t = Duration::from_millis(300);
    let msg = |e: chromiumoxide::Error| e.to_string();
    assert_eq!(
        msg(expect(&page.locator("#a"))
            .to_have_text("nope")
            .timeout(t)
            .await
            .unwrap_err()),
        "expect(locator).toHaveText(expected) failed\n\nLocator:  locator('#a')\nExpected: \"nope\"\nReceived: \"Hello  world\"\nTimeout:  300ms"
    );
    assert_eq!(
        msg(expect(&page.locator("#a"))
            .not()
            .to_be_visible()
            .timeout(t)
            .await
            .unwrap_err()),
        "expect(locator).not.toBeVisible() failed\n\nLocator:  locator('#a')\nExpected: not visible\nReceived: visible\nTimeout:  300ms"
    );
    assert_eq!(
        msg(expect(&page.locator("#zz"))
            .to_be_visible()
            .timeout(t)
            .await
            .unwrap_err()),
        "expect(locator).toBeVisible() failed\n\nLocator: locator('#zz')\nExpected: visible\nTimeout: 300ms\nError: element(s) not found"
    );
    assert_eq!(
        msg(expect(&page.locator("div,input"))
            .to_have_count(3)
            .timeout(t)
            .await
            .unwrap_err()),
        "expect(locator).toHaveCount(expected) failed\n\nLocator:  locator('div,input')\nExpected: 3\nReceived: 2\nTimeout:  300ms"
    );
    assert_eq!(
        msg(expect(&page)
            .to_have_title("t")
            .timeout(t)
            .await
            .unwrap_err()),
        "expect(page).toHaveTitle(expected) failed\n\nExpected: \"t\"\nReceived: \"My Page\"\nTimeout:  300ms"
    );
    // Strict mode violations are not retried.
    let strict = msg(expect(&page.locator("li"))
        .to_have_text("One")
        .await
        .unwrap_err());
    assert!(strict.contains("strict mode violation"), "{strict}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn response_to_be_ok() {
    let server = crate::server::TestServer::start().await;
    server.route("/ok", "text/plain", "ok", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let ok = page.goto(&server.url("/ok")).await.unwrap().unwrap();
    expect(&ok).to_be_ok().await.unwrap();
    let missing = page.goto(&server.url("/missing")).await.unwrap().unwrap();
    let err = expect(&missing).to_be_ok().await.unwrap_err().to_string();
    assert!(err.starts_with("expect(response).toBeOK() failed"), "{err}");
    expect(&missing).not().to_be_ok().await.unwrap();
    browser.close().await.unwrap();
}
