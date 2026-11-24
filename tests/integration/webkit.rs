//! The WebKit engine (`chromiumoxide::webkit`), against Playwright's WebKit
//! build (`webkit-<revision>/pw_run.sh`), over its `--inspector-pipe`.

use chromiumoxide::WebKit;
use chromiumoxide::webkit::WebKitLaunchOptions;

/// Playwright's WebKit, or `None` (tests skip) unless
/// `CHROMIUMOXIDE_REQUIRE_WEBKIT` is set.
pub fn webkit_executable() -> Option<std::path::PathBuf> {
    let exe = if cfg!(windows) {
        "Playwright.exe"
    } else {
        "pw_run.sh"
    };
    let found = crate::newest_playwright_install("webkit-", exe);
    if found.is_none() && std::env::var_os("CHROMIUMOXIDE_REQUIRE_WEBKIT").is_some() {
        panic!("no Playwright WebKit installed (CHROMIUMOXIDE_REQUIRE_WEBKIT is set)");
    }
    found
}

pub fn options() -> Option<WebKitLaunchOptions> {
    Some(WebKitLaunchOptions {
        executable_path: Some(webkit_executable()?),
        ..Default::default()
    })
}

#[tokio::test]
async fn webkit_launches_and_runs_a_page() {
    let Some(options) = options() else {
        return;
    };
    let browser = WebKit::launch(options).await.unwrap();
    assert!(!browser.version().await.unwrap().is_empty());
    let page = browser.new_page().await.unwrap();
    page.goto("data:text/html,<title>Kit</title><h1>Hello WebKit</h1>")
        .await
        .unwrap();
    assert_eq!(page.title().await.unwrap(), "Kit");
    assert_eq!(page.evaluate::<i64>("6 * 7").await.unwrap(), 42);
    assert_eq!(
        page.evaluate::<String>("document.querySelector('h1').textContent")
            .await
            .unwrap(),
        "Hello WebKit"
    );
    // Playwright's default user agent for its WebKit.
    let ua: String = page.evaluate("navigator.userAgent").await.unwrap();
    assert!(
        ua.contains("AppleWebKit/605.1.15") && ua.contains("Safari"),
        "{ua}"
    );
    browser.close().await.unwrap();
}

/// Context emulation, set_content, add_style_tag, resize and screenshots.
#[tokio::test]
async fn webkit_contexts_emulate_devices() {
    use chromiumoxide::{ContextOptions, StyleTag, Viewport};
    let Some(options) = options() else {
        return;
    };
    let browser = WebKit::launch(options).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(390, 664)),
            screen: Some(Viewport::new(390, 844)),
            device_scale_factor: Some(3.0),
            user_agent: Some("CoxTest/1.0".into()),
            locale: Some("en-GB".into()),
            timezone_id: Some("Asia/Tokyo".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.set_content("<p id=p>hi</p>").await.unwrap();
    let seen: serde_json::Value = page
        .evaluate(
            "({ w: innerWidth, h: innerHeight, dpr: devicePixelRatio, ua: navigator.userAgent,
               lang: navigator.language, tz: Intl.DateTimeFormat().resolvedOptions().timeZone,
               sw: screen.width, sh: screen.height })",
        )
        .await
        .unwrap();
    assert_eq!(
        seen,
        serde_json::json!({"w": 390, "h": 664, "dpr": 3, "ua": "CoxTest/1.0", "lang": "en-GB",
                           "tz": "Asia/Tokyo", "sw": 390, "sh": 844})
    );
    page.add_style_tag(StyleTag::content("#p { color: rgb(1, 2, 3) }"))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<String>("getComputedStyle(document.getElementById('p')).color")
            .await
            .unwrap(),
        "rgb(1, 2, 3)"
    );
    let png = page.screenshot().await.unwrap();
    let (w, h, _) = crate::screenshot::decode(&png);
    assert_eq!((w, h), (390 * 3, 664 * 3));
    let full = page.screenshot().full_page(true).await.unwrap();
    assert_eq!(crate::screenshot::decode(&full).0, 390 * 3);
    page.set_viewport_size(Viewport::new(500, 400))
        .await
        .unwrap();
    assert_eq!(page.evaluate::<i64>("innerWidth").await.unwrap(), 500);
    page.wait_for_function("innerWidth === 500").await.unwrap();
    context.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn webkit_locators_query_the_page() {
    use chromiumoxide::RoleOptions;
    let Some(options) = options() else {
        return;
    };
    let browser = WebKit::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<ul><li class=a>One</li><li class=a>Two</li><li class=a data-testid=three>Three</li></ul>
         <button title=go>Go <b>now</b></button><input hidden value=x>
         <section><p>inner</p></section>
         <script>setTimeout(() => document.body.insertAdjacentHTML('beforeend', '<p id=late>Late</p>'), 300)</script>",
    )
    .await
    .unwrap();
    let items = page.locator("li.a");
    assert_eq!(items.count().await.unwrap(), 3);
    assert_eq!(
        items.all_text_contents().await.unwrap(),
        ["One", "Two", "Three"]
    );
    assert_eq!(
        items.nth(1).text_content().await.unwrap().as_deref(),
        Some("Two")
    );
    assert_eq!(items.first().inner_text().await.unwrap(), "One");
    assert_eq!(items.last().inner_text().await.unwrap(), "Three");
    assert_eq!(
        page.get_by_role("button", RoleOptions::default())
            .inner_text()
            .await
            .unwrap(),
        "Go now"
    );
    assert_eq!(
        page.locator("button")
            .get_attribute("title")
            .await
            .unwrap()
            .as_deref(),
        Some("go")
    );
    assert_eq!(
        page.get_by_test_id("three")
            .text_content()
            .await
            .unwrap()
            .as_deref(),
        Some("Three")
    );
    assert!(page.get_by_text("Two", true).is_visible().await.unwrap());
    assert!(!page.locator("input").is_visible().await.unwrap());
    assert_eq!(
        page.locator("section")
            .locator("p")
            .inner_html()
            .await
            .unwrap(),
        "inner"
    );
    // Reads wait for the element, like Playwright.
    assert_eq!(page.locator("#late").inner_text().await.unwrap(), "Late");
    let err = page.locator("li").text_content().await.unwrap_err();
    assert!(err.to_string().contains("strict mode violation"), "{err}");
    browser.close().await.unwrap();
}

/// Input over WebKit's `Input.*` commands: click, fill, press (named keys
/// and chords), press_sequentially, check/uncheck, hover.
#[tokio::test]
async fn webkit_locators_act_on_elements() {
    let Some(options) = options() else {
        return;
    };
    let browser = WebKit::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<div style='height: 1500px'>spacer</div>
         <button onclick=\"this.textContent = 'clicked ' + event.detail\">Go</button>
         <input id=i><input type=checkbox id=c>
         <div id=h onmouseover=\"this.textContent = 'hovered'\">hover me</div>
         <input id=k onkeydown=\"this.dataset.keys = (this.dataset.keys || '') + event.key + ','\">",
    )
    .await
    .unwrap();
    // Below the fold: click scrolls it into view first.
    page.locator("button").click().await.unwrap();
    assert_eq!(
        page.locator("button").inner_text().await.unwrap(),
        "clicked 1"
    );

    let input = page.locator("#i");
    input.fill("hello").await.unwrap();
    assert_eq!(input.input_value().await.unwrap(), "hello");
    input.press("Backspace").await.unwrap();
    assert_eq!(input.input_value().await.unwrap(), "hell");
    input.press("Control+a").await.unwrap();
    input.press_sequentially("yo!").await.unwrap();
    assert_eq!(input.input_value().await.unwrap(), "yo!");
    input.fill("").await.unwrap();
    assert_eq!(input.input_value().await.unwrap(), "");

    let check = page.locator("#c");
    check.check().await.unwrap();
    assert!(
        page.evaluate::<bool>("document.getElementById('c').checked")
            .await
            .unwrap()
    );
    check.uncheck().await.unwrap();
    assert!(
        !page
            .evaluate::<bool>("document.getElementById('c').checked")
            .await
            .unwrap()
    );

    page.locator("#h").hover().await.unwrap();
    assert_eq!(page.locator("#h").inner_text().await.unwrap(), "hovered");

    page.locator("#k").press("Enter").await.unwrap();
    page.locator("#k").press("ArrowLeft").await.unwrap();
    assert_eq!(
        page.locator("#k")
            .get_attribute("data-keys")
            .await
            .unwrap()
            .as_deref(),
        Some("Enter,ArrowLeft,")
    );
    browser.close().await.unwrap();
}

/// History (back, forward, reload), cookies, init scripts and page events
/// (console, uncaught errors, dialogs) on WebKit.
#[tokio::test]
async fn webkit_history_cookies_scripts_and_events() {
    use chromiumoxide::webkit::PageEvent;
    use chromiumoxide::{ContextOptions, NewCookie};
    use futures::StreamExt;
    use std::time::Duration;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route(
        "/one.html",
        "text/html",
        "<title>one</title><a href='/two.html'>next</a>",
        0,
    );
    server.route("/two.html", "text/html", "<title>two</title>", 0);
    let browser = WebKit::launch(options).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();

    // Cookies, per context.
    let url = server.url("/one.html");
    context
        .add_cookies(&[NewCookie::new("flavour", "oat").url(&url)])
        .await
        .unwrap();
    let cookies = context.cookies().await.unwrap();
    assert_eq!(cookies.len(), 1, "{cookies:?}");
    assert_eq!(
        (cookies[0].name.as_str(), cookies[0].value.as_str()),
        ("flavour", "oat")
    );
    assert_eq!(cookies[0].domain, "127.0.0.1");
    assert!(
        context
            .cookies_for(&["https://example.com"])
            .await
            .unwrap()
            .is_empty()
    );

    // Init scripts, for the context and for one page.
    context
        .add_init_script("window.fromInit = 7; var initVar = 8;")
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.add_init_script("window.pageInit = 'yes'")
        .await
        .unwrap();
    page.goto(&url).await.unwrap();
    assert_eq!(
        page.evaluate::<String>("document.cookie").await.unwrap(),
        "flavour=oat"
    );
    assert_eq!(page.evaluate::<i64>("window.fromInit").await.unwrap(), 7);
    assert_eq!(page.evaluate::<i64>("window.initVar").await.unwrap(), 8);
    assert_eq!(
        page.evaluate::<String>("window.pageInit").await.unwrap(),
        "yes"
    );

    // History.
    page.locator("a").click().await.unwrap();
    page.wait_for_function("document.title === 'two'")
        .await
        .unwrap();
    assert!(page.url().ends_with("/two.html"), "{}", page.url());
    page.go_back().await.unwrap();
    assert_eq!(page.title().await.unwrap(), "one");
    assert!(page.url().ends_with("/one.html"), "{}", page.url());
    page.go_forward().await.unwrap();
    assert_eq!(page.title().await.unwrap(), "two");
    page.evaluate::<String>("document.body.dataset.mark = 'x'")
        .await
        .unwrap();
    page.reload().await.unwrap();
    assert_eq!(
        page.evaluate::<Option<String>>("document.body.dataset.mark ?? null")
            .await
            .unwrap(),
        None
    );

    // Events.
    let mut events = page.events();
    let mut next = async || loop {
        let e = tokio::time::timeout(Duration::from_secs(10), events.next())
            .await
            .expect("event within 10 s")
            .expect("stream open");
        if !matches!(e, PageEvent::Load | PageEvent::DomContentLoaded) {
            return e;
        }
    };
    page.evaluate::<()>("console.log('hello', 42); console.warn('careful')")
        .await
        .unwrap();
    match next().await {
        PageEvent::Console(m) => {
            assert_eq!(m.kind(), "log");
            assert_eq!(m.text(), "hello 42");
        }
        other => panic!("{other:?}"),
    }
    match next().await {
        PageEvent::Console(m) => {
            assert_eq!(m.kind(), "warning");
            assert_eq!(m.text(), "careful");
        }
        other => panic!("{other:?}"),
    }
    page.evaluate::<()>("void setTimeout(() => { throw new Error('boom') })")
        .await
        .unwrap();
    match next().await {
        PageEvent::PageError(e) => {
            assert_eq!(e.message(), "boom");
            assert_eq!(e.name(), "Error");
        }
        other => panic!("{other:?}"),
    }
    let answer = {
        let page = page.clone();
        tokio::spawn(async move { page.evaluate::<String>("prompt('name?', 'anon')").await })
    };
    match next().await {
        PageEvent::Dialog(d) => {
            assert_eq!(d.kind(), "prompt");
            assert_eq!(d.message(), "name?");
            assert_eq!(d.default_value(), "anon");
            d.accept(Some("Kit")).await.unwrap();
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(answer.await.unwrap().unwrap(), "Kit");
    // Nobody listening: Playwright dismisses the dialog.
    drop(events);
    assert!(!page.evaluate::<bool>("confirm('ok?')").await.unwrap());

    // Another context does not see the cookies; clearing empties them.
    let other = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    assert!(other.cookies().await.unwrap().is_empty());
    context.clear_cookies().await.unwrap();
    assert!(context.cookies().await.unwrap().is_empty());
    browser.close().await.unwrap();
}
