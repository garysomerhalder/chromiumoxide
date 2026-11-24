//! The Firefox engine (`chromiumoxide::firefox`, over WebDriver BiDi)
//! against Playwright's Firefox build.

use chromiumoxide::Firefox;
use chromiumoxide::firefox::FirefoxLaunchOptions;

/// Playwright's newest Firefox (`firefox-<revision>/firefox/firefox`). With
/// `CHROMIUMOXIDE_REQUIRE_FIREFOX` set, a missing Firefox fails the test
/// instead of skipping it.
pub fn firefox_executable() -> Option<std::path::PathBuf> {
    let found = crate::newest_playwright_install("firefox-", "firefox/firefox");
    if found.is_none() && std::env::var_os("CHROMIUMOXIDE_REQUIRE_FIREFOX").is_some() {
        panic!("no Playwright Firefox installed (CHROMIUMOXIDE_REQUIRE_FIREFOX is set)");
    }
    found
}

fn options() -> Option<FirefoxLaunchOptions> {
    Some(FirefoxLaunchOptions {
        executable_path: Some(firefox_executable()?),
        ..Default::default()
    })
}

#[tokio::test]
async fn firefox_launches_and_runs_a_page() {
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
    let version = browser.version().await.unwrap();
    assert!(version.starts_with("159."), "{version}");
    let page = browser.new_page().await.unwrap();
    page.goto("data:text/html,<title>Fox</title><h1>Hello Firefox</h1>")
        .await
        .unwrap();
    assert_eq!(page.title().await.unwrap(), "Fox");
    assert_eq!(page.evaluate::<i64>("6 * 7").await.unwrap(), 42);
    assert_eq!(
        page.evaluate::<String>("document.querySelector('h1').textContent")
            .await
            .unwrap(),
        "Hello Firefox"
    );
    browser.close().await.unwrap();
}

/// Per-context emulation over BiDi (viewport, device scale factor, user
/// agent, locale, timezone, screen), set_content, add_style_tag, resize and
/// screenshot sizes.
#[tokio::test]
async fn firefox_contexts_emulate_devices() {
    use chromiumoxide::{ContextOptions, StyleTag, Viewport};
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
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
    page.set_viewport_size(Viewport::new(500, 400))
        .await
        .unwrap();
    assert_eq!(page.evaluate::<i64>("innerWidth").await.unwrap(), 500);

    // Not over BiDi in Firefox 159: a clear error instead of a silent no-op.
    let err = browser
        .new_context(ContextOptions {
            color_scheme: Some(chromiumoxide::ColorScheme::Dark),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Firefox"), "{err}");
    context.close().await.unwrap();
    browser.close().await.unwrap();
}

/// Locators run Playwright's injected script in a BiDi sandbox: CSS, role,
/// text and test-id selectors, nth/first/last, chaining, reads, visibility,
/// waiting for late elements and strictness.
#[tokio::test]
async fn firefox_locators_query_the_page() {
    use chromiumoxide::RoleOptions;
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
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

/// Input over BiDi `input.performActions`: click, fill, press (named keys
/// and chords), press_sequentially, check/uncheck, hover.
#[tokio::test]
async fn firefox_locators_act_on_elements() {
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
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

/// Navigation and waits: url (tracked from BiDi events, also for
/// navigations the page starts), content, reload, back/forward,
/// wait_for_function, wait_for_timeout and locator.wait_for.
#[tokio::test]
async fn firefox_navigates_and_waits() {
    use chromiumoxide::WaitForState;
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
    server.route(
        "/two.html",
        "text/html",
        "<title>two</title><p>second</p>",
        0,
    );
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/one.html")).await.unwrap();
    assert!(page.url().ends_with("/one.html"), "{}", page.url());
    assert!(page.content().await.unwrap().contains("<title>one</title>"));

    // A navigation the page starts itself.
    page.locator("a").click().await.unwrap();
    page.wait_for_function("document.title === 'two'")
        .await
        .unwrap();
    assert!(page.url().ends_with("/two.html"), "{}", page.url());

    page.go_back().await.unwrap();
    page.wait_for_function("document.title === 'one'")
        .await
        .unwrap();
    assert!(page.url().ends_with("/one.html"), "{}", page.url());
    page.go_forward().await.unwrap();
    page.wait_for_function("document.title === 'two'")
        .await
        .unwrap();

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

    // wait_for_function returns the first truthy value.
    page.evaluate::<()>("void setTimeout(() => { window.answer = 42 }, 200)")
        .await
        .unwrap();
    let v = page.wait_for_function("window.answer").await.unwrap();
    assert_eq!(v, serde_json::json!(42));
    page.wait_for_timeout(std::time::Duration::from_millis(50))
        .await;

    page.set_content(
        "<p id=p hidden>hi</p><script>setTimeout(() => p.hidden = false, 200)</script>",
    )
    .await
    .unwrap();
    page.locator("#p")
        .wait_for(WaitForState::Attached)
        .await
        .unwrap();
    page.locator("#p")
        .wait_for(WaitForState::Visible)
        .await
        .unwrap();
    assert!(page.locator("#p").is_visible().await.unwrap());
    page.evaluate::<()>("p.remove()").await.unwrap();
    page.locator("#p")
        .wait_for(WaitForState::Detached)
        .await
        .unwrap();
    page.locator("#p")
        .wait_for(WaitForState::Hidden)
        .await
        .unwrap();
    browser.close().await.unwrap();
}

/// Web-first assertions on Firefox: the same `expect()` entry point and
/// Playwright's injected checks, retried until they match or time out.
#[tokio::test]
async fn firefox_expect_assertions() {
    use chromiumoxide::expect;
    use std::time::Duration;
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<title>Fox page</title><h1 id=t class='big title'>Loading</h1>
         <ul><li>a</li><li>b</li></ul><input id=v value=val><input type=checkbox id=c checked>
         <p id=gone hidden>x</p>
         <script>setTimeout(() => t.textContent = 'Ready', 300)</script>",
    )
    .await
    .unwrap();
    let h1 = page.locator("h1");
    expect(&h1).to_have_text("Ready").await.unwrap();
    expect(&h1).to_contain_text("ead").await.unwrap();
    expect(&h1).to_have_attribute("id", "t").await.unwrap();
    expect(&h1).to_contain_class("title").await.unwrap();
    expect(&h1).to_be_visible().await.unwrap();
    expect(&page.locator("#gone"))
        .not()
        .to_be_visible()
        .await
        .unwrap();
    expect(&page.locator("#gone")).to_be_hidden().await.unwrap();
    expect(&page.locator("li")).to_have_count(2).await.unwrap();
    expect(&page.locator("li"))
        .to_have_texts(["a", "b"])
        .await
        .unwrap();
    expect(&page.locator("#v"))
        .to_have_value("val")
        .await
        .unwrap();
    expect(&page.locator("#c")).to_be_checked().await.unwrap();
    expect(&page).to_have_title("Fox page").await.unwrap();

    let err = expect(&h1)
        .to_have_text("Never")
        .timeout(Duration::from_millis(300))
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("toHaveText"), "{message}");
    assert!(message.contains("\"Never\""), "{message}");
    assert!(message.contains("\"Ready\""), "{message}");
    browser.close().await.unwrap();
}

/// Page events on Firefox: console messages, uncaught errors and dialogs
/// (accepted with text; dismissed automatically when nobody listens).
#[tokio::test]
async fn firefox_page_events() {
    use chromiumoxide::firefox::PageEvent;
    use futures::StreamExt;
    use std::time::Duration;
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
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
            d.accept(Some("Fox")).await.unwrap();
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(answer.await.unwrap().unwrap(), "Fox");

    // Nobody listening: Playwright dismisses the dialog.
    drop(events);
    assert!(!page.evaluate::<bool>("confirm('ok?')").await.unwrap());
    browser.close().await.unwrap();
}

/// Cookies (per context, over BiDi `storage.*`) and init scripts (BiDi
/// preload scripts, with script semantics: top-level `var` is global).
#[tokio::test]
async fn firefox_cookies_and_init_scripts() {
    use chromiumoxide::{ContextOptions, NewCookie};
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/c.html", "text/html", "<p>c</p>", 0);
    let url = server.url("/c.html");
    let browser = Firefox::launch(options).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
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
    assert_eq!(cookies[0].path, "/");
    assert!(
        context
            .cookies_for(&["https://example.com"])
            .await
            .unwrap()
            .is_empty()
    );

    context
        .add_init_script("window.fromInit = 7; var initVar = 8;")
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&url).await.unwrap();
    assert_eq!(
        page.evaluate::<String>("document.cookie").await.unwrap(),
        "flavour=oat"
    );
    assert_eq!(page.evaluate::<i64>("window.fromInit").await.unwrap(), 7);
    assert_eq!(page.evaluate::<i64>("window.initVar").await.unwrap(), 8);

    page.add_init_script("window.pageInit = 'yes'")
        .await
        .unwrap();
    page.reload().await.unwrap();
    assert_eq!(
        page.evaluate::<String>("window.pageInit").await.unwrap(),
        "yes"
    );

    // Another context does not see these cookies.
    let other = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    assert!(other.cookies().await.unwrap().is_empty());

    context.clear_cookies().await.unwrap();
    assert!(context.cookies().await.unwrap().is_empty());
    browser.close().await.unwrap();
}

/// page.route on Firefox (BiDi network intercepts): fulfil with a made-up
/// response, continue, abort; the newest matching handler wins; unroute.
#[tokio::test]
async fn firefox_routes_requests() {
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/r.html", "text/html", "<p>r</p>", 0);
    server.route("/pass.txt", "text/plain", "from the server", 0);
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.route("**/api/*", |route| async move {
        assert_eq!(route.request().method(), "GET");
        route
            .fulfill()
            .status(201)
            .content_type("application/json")
            .header("x-fake", "yes")
            .body(r#"{"fake":true}"#)
            .await
            .unwrap();
    })
    .await
    .unwrap();
    page.route("**/pass.txt", |route| async move {
        assert!(route.request().url().ends_with("/pass.txt"));
        route.continue_().await.unwrap();
    })
    .await
    .unwrap();
    page.route("**/blocked", |route| async move {
        route.abort().await.unwrap();
    })
    .await
    .unwrap();
    page.goto(&server.url("/r.html")).await.unwrap();

    let fake: serde_json::Value = page
        .evaluate(
            "fetch('/api/x').then(async r => ({ status: r.status, header: r.headers.get('x-fake'), body: await r.json() }))",
        )
        .await
        .unwrap();
    assert_eq!(
        fake,
        serde_json::json!({"status": 201, "header": "yes", "body": {"fake": true}})
    );
    assert_eq!(
        page.evaluate::<String>("fetch('/pass.txt').then(r => r.text())")
            .await
            .unwrap(),
        "from the server"
    );
    assert_eq!(
        page.evaluate::<String>("fetch('/blocked').then(() => 'loaded', () => 'failed')")
            .await
            .unwrap(),
        "failed"
    );

    // Newest handler first; unroute removes it again.
    page.route("**/api/*", |route| async move {
        route.fulfill().body("newer").await.unwrap();
    })
    .await
    .unwrap();
    assert_eq!(
        page.evaluate::<String>("fetch('/api/y').then(r => r.text())")
            .await
            .unwrap(),
        "newer"
    );
    page.unroute("**/api/*").await.unwrap();
    assert_eq!(
        page.evaluate::<u16>("fetch('/api/z').then(r => r.status)")
            .await
            .unwrap(),
        404
    );
    browser.close().await.unwrap();
}

/// Network on Firefox: context.route (for every page of the context,
/// page routes first), request / response / requestfinished /
/// requestfailed events, and HTTP credentials (httpCredentials).
#[tokio::test]
async fn firefox_network_events_context_routes_and_auth() {
    use chromiumoxide::firefox::PageEvent;
    use chromiumoxide::{ContextOptions, HttpCredentials};
    use futures::StreamExt;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/n.html", "text/html", "<p>n</p>", 0);
    server.route("/data.txt", "text/plain", "real data", 0);
    let browser = Firefox::launch(options).await.unwrap();
    let context = browser
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
    context
        .route("**/ctx/*", |route| async move {
            route.fulfill().body("from the context").await.unwrap();
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    let mut events = page.events();
    page.goto(&server.url("/n.html")).await.unwrap();
    assert_eq!(
        page.evaluate::<String>("fetch('/ctx/a').then(r => r.text())")
            .await
            .unwrap(),
        "from the context"
    );
    // A page route wins over the context's.
    page.route("**/ctx/b", |route| async move {
        route.fulfill().body("from the page").await.unwrap();
    })
    .await
    .unwrap();
    assert_eq!(
        page.evaluate::<String>("fetch('/ctx/b').then(r => r.text())")
            .await
            .unwrap(),
        "from the page"
    );
    // Context routes also apply to pages opened later.
    let second = context.new_page().await.unwrap();
    second.goto(&server.url("/n.html")).await.unwrap();
    assert_eq!(
        second
            .evaluate::<String>("fetch('/ctx/c').then(r => r.text())")
            .await
            .unwrap(),
        "from the context"
    );

    // Request / response events.
    assert_eq!(
        page.evaluate::<String>("fetch('/data.txt').then(r => r.text())")
            .await
            .unwrap(),
        "real data"
    );
    // A closed port (port 1 would be refused by Firefox before any request).
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let _ = page
        .evaluate_with::<String, _>(
            "port => fetch(`http://127.0.0.1:${port}/nope`).then(() => 'ok', () => 'failed')",
            closed,
        )
        .await
        .unwrap();
    let closed_url = format!("127.0.0.1:{closed}/nope");
    let (mut request, mut response, mut finished, mut failed) = (false, false, false, false);
    while !(request && response && finished && failed) {
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.next())
            .await
            .expect("network events within 10 s")
            .unwrap();
        match event {
            PageEvent::Request(r) if r.url().ends_with("/data.txt") => {
                assert_eq!(r.method(), "GET");
                request = true;
            }
            PageEvent::Response(r) if r.url().ends_with("/data.txt") => {
                assert_eq!(r.status(), 200);
                assert_eq!(r.header_value("content-type"), Some("text/plain"));
                response = true;
            }
            PageEvent::RequestFinished(r) if r.url().ends_with("/data.txt") => finished = true,
            PageEvent::RequestFailed(r) if r.url().contains(&closed_url) => {
                assert!(r.failure().is_some());
                failed = true;
            }
            _ => {}
        }
    }

    // httpCredentials answer the server's Basic challenge.
    page.goto(&server.url("/protected")).await.unwrap();
    assert_eq!(
        page.locator("body").inner_text().await.unwrap(),
        "secret ok"
    );
    browser.close().await.unwrap();
}

/// Downloads on Firefox: `PageEvent::Download`, the suggested file name,
/// waiting for the file, reading it and saving a copy.
#[tokio::test]
async fn firefox_downloads() {
    use chromiumoxide::firefox::PageEvent;
    use futures::StreamExt;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.respond(
        "/dl",
        "200 OK",
        &[("Content-Disposition", "attachment; filename=f.txt")],
        "downloaded bytes",
    );
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let mut events = page.events();
    page.set_content(&format!("<a href='{}'>dl</a>", server.url("/dl")))
        .await
        .unwrap();
    page.locator("a").click().await.unwrap();
    let download = loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.next())
            .await
            .expect("download within 10 s")
            .unwrap();
        if let PageEvent::Download(d) = event {
            break d;
        }
    };
    assert_eq!(download.suggested_filename(), "f.txt");
    assert!(download.url().ends_with("/dl"));
    assert_eq!(download.read().await.unwrap(), b"downloaded bytes");
    assert_eq!(download.failure().await.unwrap(), None);
    let copy = std::env::temp_dir().join(format!("cox-ff-dl-{}.txt", std::process::id()));
    download.save_as(&copy).await.unwrap();
    assert_eq!(std::fs::read(&copy).unwrap(), b"downloaded bytes");
    std::fs::remove_file(&copy).unwrap();
    browser.close().await.unwrap();
}
