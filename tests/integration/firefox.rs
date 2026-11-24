//! The Firefox engine (`chromiumoxide::firefox`, over WebDriver BiDi)
//! against Playwright's Firefox build.

use chromiumoxide::Firefox;
use chromiumoxide::firefox::FirefoxLaunchOptions;

/// Playwright's newest Firefox (`firefox-<revision>/firefox/firefox`, the
/// `Nightly.app` on macOS, `firefox.exe` on Windows). With
/// `CHROMIUMOXIDE_REQUIRE_FIREFOX` set, a missing Firefox fails the test
/// instead of skipping it.
pub fn firefox_executable() -> Option<std::path::PathBuf> {
    let exe = if cfg!(windows) {
        "firefox/firefox.exe"
    } else if cfg!(target_os = "macos") {
        "firefox/Nightly.app/Contents/MacOS/firefox"
    } else {
        "firefox/firefox"
    };
    let found = crate::newest_playwright_install("firefox-", exe);
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
    input.press("ControlOrMeta+a").await.unwrap();
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

/// Iframes on Firefox: frame_locator (reads, clicks and fills inside an
/// offset iframe, assertions), page.frames() and frame.evaluate.
#[tokio::test]
async fn firefox_frames() {
    use chromiumoxide::expect;
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<p>top</p><iframe id=f style='margin: 60px 0 0 80px; border: 7px solid red; padding: 5px'
           srcdoc=\"<button onclick='this.textContent=&quot;clicked&quot;'>in frame</button><input id=i>\"></iframe>",
    )
    .await
    .unwrap();
    let frame = page.frame_locator("#f");
    assert_eq!(
        frame.locator("button").inner_text().await.unwrap(),
        "in frame"
    );
    frame.locator("button").click().await.unwrap();
    expect(&frame.get_by_text("clicked", true))
        .to_be_visible()
        .await
        .unwrap();
    frame.locator("#i").fill("typed").await.unwrap();
    assert_eq!(frame.locator("#i").input_value().await.unwrap(), "typed");
    assert_eq!(page.locator("button").count().await.unwrap(), 0);

    let frames = page.frames().await.unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].url(), "about:srcdoc");
    assert_eq!(
        frames[1]
            .evaluate::<String>("document.querySelector('button').textContent")
            .await
            .unwrap(),
        "clicked"
    );
    browser.close().await.unwrap();
}

/// Saving and restoring a logged-in state on Firefox: storage_state()
/// collects cookies and localStorage; a new context with that state sees
/// both before any page script runs.
#[tokio::test]
async fn firefox_storage_state_round_trip() {
    use chromiumoxide::ContextOptions;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/s.html", "text/html", "<p>s</p>", 0);
    server.route(
        "/check.html",
        "text/html",
        "<script>window.seen = localStorage.getItem('token')</script>",
        0,
    );
    let url = server.url("/s.html");
    let origin = url.trim_end_matches("/s.html").to_owned();
    let browser = Firefox::launch(options).await.unwrap();
    let first = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let page = first.new_page().await.unwrap();
    page.goto(&url).await.unwrap();
    page.evaluate::<()>(
        "localStorage.setItem('token', 'abc'); void (document.cookie = 'session=s1')",
    )
    .await
    .unwrap();
    let state = first.storage_state().await.unwrap();
    assert!(
        state
            .cookies
            .iter()
            .any(|c| c.name == "session" && c.value == "s1")
    );
    let saved = state
        .origins
        .iter()
        .find(|o| o.origin == origin)
        .expect("origin saved");
    assert!(
        saved
            .local_storage
            .iter()
            .any(|kv| kv.name == "token" && kv.value == "abc")
    );

    let second = browser
        .new_context(ContextOptions {
            storage_state: Some(state),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = second.new_page().await.unwrap();
    page.goto(&server.url("/check.html")).await.unwrap();
    assert_eq!(page.evaluate::<String>("window.seen").await.unwrap(), "abc");
    assert_eq!(
        page.evaluate::<String>("document.cookie").await.unwrap(),
        "session=s1"
    );
    browser.close().await.unwrap();
}

/// More input on Firefox: dblclick, drag_to, select_option,
/// set_input_files, and the raw page.mouse / page.keyboard.
#[tokio::test]
async fn firefox_more_input() {
    use chromiumoxide::MouseButton;
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<button id=d ondblclick=\"this.textContent = 'double'\">twice</button>
         <div id=src draggable=true style='width:50px;height:50px;background:red'>drag</div>
         <div id=dst style='width:100px;height:100px;background:blue'
              ondragover='event.preventDefault()' ondrop=\"this.textContent = 'dropped'\">drop here</div>
         <select id=s multiple><option value=a>Apple</option><option value=b>Banana</option><option value=c>Cherry</option></select>
         <input type=file id=f multiple>
         <textarea id=t></textarea>
         <div id=m style='width:200px;height:100px' onmousedown=\"this.dataset.log = (this.dataset.log||'') + 'down' + event.button + ','\"
              onmouseup=\"this.dataset.log += 'up,'\" onwheel=\"this.dataset.wheel = event.deltaY\">mouse</div>",
    )
    .await
    .unwrap();

    page.locator("#d").dblclick().await.unwrap();
    assert_eq!(page.locator("#d").inner_text().await.unwrap(), "double");

    page.locator("#src")
        .drag_to(&page.locator("#dst"))
        .await
        .unwrap();
    assert_eq!(page.locator("#dst").inner_text().await.unwrap(), "dropped");

    let picked = page
        .locator("#s")
        .select_option(&["a", "Cherry"])
        .await
        .unwrap();
    assert_eq!(picked, ["a", "c"]);
    assert_eq!(
        page.evaluate::<Vec<String>>(
            "[...document.getElementById('s').selectedOptions].map(o => o.value)"
        )
        .await
        .unwrap(),
        ["a", "c"]
    );

    let dir = std::env::temp_dir().join(format!("cox-ff-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (one, two) = (dir.join("one.txt"), dir.join("two.txt"));
    std::fs::write(&one, "1").unwrap();
    std::fs::write(&two, "22").unwrap();
    page.locator("#f")
        .set_input_files(&[one, two])
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<Vec<String>>(
            "[...document.getElementById('f').files].map(f => f.name + ':' + f.size)"
        )
        .await
        .unwrap(),
        ["one.txt:1", "two.txt:2"]
    );
    std::fs::remove_dir_all(&dir).unwrap();

    page.locator("#t").focus().await.unwrap();
    let keyboard = page.keyboard();
    keyboard.type_text("Hi").await.unwrap();
    keyboard.down("Shift").await.unwrap();
    keyboard.press("a").await.unwrap();
    keyboard.up("Shift").await.unwrap();
    keyboard.insert_text(" there").await.unwrap();
    keyboard.press("Backspace").await.unwrap();
    assert_eq!(page.locator("#t").input_value().await.unwrap(), "HiA ther");

    let mouse = page.mouse();
    let b = page
        .evaluate::<Vec<f64>>(
            "(r => [r.x, r.y])(document.getElementById('m').getBoundingClientRect())",
        )
        .await
        .unwrap();
    mouse.move_to(b[0] + 20.0, b[1] + 20.0, 3).await.unwrap();
    mouse.down(MouseButton::Left).await.unwrap();
    mouse.up(MouseButton::Left).await.unwrap();
    mouse.click(b[0] + 30.0, b[1] + 30.0).await.unwrap();
    mouse.wheel(0.0, 120.0).await.unwrap();
    page.wait_for_function("document.getElementById('m').dataset.wheel")
        .await
        .unwrap();
    assert_eq!(
        page.locator("#m")
            .get_attribute("data-log")
            .await
            .unwrap()
            .as_deref(),
        Some("down0,up,down0,up,")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn firefox_pdf_and_har() {
    use chromiumoxide::{ContextOptions, RecordHar};
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/page", "text/html", "<p>page</p>", 0);
    let dir = std::env::temp_dir().join(format!("cox-ff-har-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let har_path = dir.join("out.har");
    let browser = Firefox::launch(options).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            record_har: Some(RecordHar::new(&har_path)),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/page")).await.unwrap();
    let echoed = page
        .evaluate::<String>("fetch('/echo', {method: 'POST', body: 'x=1'}).then(r => r.text())")
        .await
        .unwrap();
    assert!(!echoed.is_empty());

    let pdf_path = dir.join("page.pdf");
    let pdf = page.pdf().path(&pdf_path).await.unwrap();
    assert_eq!(&pdf[..5], b"%PDF-");
    assert_eq!(std::fs::read(&pdf_path).unwrap(), pdf);

    context.close().await.unwrap();
    let har: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&har_path).unwrap()).unwrap();
    let log = &har["log"];
    assert_eq!(log["version"], "1.2");
    assert_eq!(log["browser"]["name"], "firefox");
    let pages = log["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 1);
    let entries = log["entries"].as_array().unwrap();
    let find = |p: &str| {
        entries
            .iter()
            .find(|e| e["request"]["url"] == server.url(p))
            .unwrap_or_else(|| panic!("no entry for {p}: {entries:#?}"))
    };
    let doc = find("/page");
    assert_eq!(doc["pageref"], pages[0]["id"]);
    assert_eq!(doc["request"]["method"], "GET");
    assert_eq!(doc["response"]["status"], 200);
    assert_eq!(doc["response"]["content"]["text"], "<p>page</p>");
    assert_eq!(doc["response"]["content"]["mimeType"], "text/html");
    assert!(doc["startedDateTime"].as_str().unwrap().ends_with('Z'));
    let post = find("/echo");
    assert_eq!(post["request"]["method"], "POST");
    assert_eq!(post["request"]["postData"]["text"], "x=1");
    std::fs::remove_dir_all(&dir).unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn firefox_records_video() {
    use chromiumoxide::{ContextOptions, RecordVideo};
    let Some(options) = options() else {
        return;
    };
    if chromiumoxide::adapters::video::find_ffmpeg().is_none() {
        eprintln!("no ffmpeg; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("cox-ff-video-{}", std::process::id()));
    let browser = Firefox::launch(options).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            record_video: Some(RecordVideo::new(&dir)),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.set_content(
        "<style>@keyframes c{from{background:red}to{background:blue}} body{animation:c .3s infinite}</style><h1>video</h1>",
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let video = page.video().expect("recording");
    page.close().await.unwrap();
    let path = video.path().await.unwrap();
    assert!(path.starts_with(&dir), "{path:?}");
    assert_eq!(path.extension().unwrap(), "webm");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], &[0x1a, 0x45, 0xdf, 0xa3], "EBML header");
    assert!(bytes.len() > 1000, "{} bytes", bytes.len());
    context.close().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn firefox_tracing() {
    use chromiumoxide::TracingStart;
    use serde_json::Value;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/page", "text/html", "<p>traced</p>", 0);
    let browser = Firefox::launch(options).await.unwrap();
    let context = browser.new_context(Default::default()).await.unwrap();
    context
        .tracing()
        .start(TracingStart {
            screenshots: true,
            title: Some("ff trace".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.set_content("<button onclick=\"console.log(1)\">B</button>")
        .await
        .unwrap();
    page.locator("button").click().await.unwrap();
    page.goto(&server.url("/page")).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let dir = std::env::temp_dir().join(format!("cox-ff-trace-{}", std::process::id()));
    let path = dir.join("trace.zip");
    context.tracing().stop(Some(&path)).await.unwrap();

    let files = crate::tracing::unzip(&path);
    let names: Vec<String> = files.as_object().unwrap().keys().cloned().collect();
    for required in ["trace.trace", "trace.network", "trace.stacks"] {
        assert!(names.iter().any(|n| n == required), "{names:?}");
    }
    let events: Vec<Value> = String::from_utf8(crate::tracing::file(&files, "trace.trace"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(events[0]["type"], "context-options");
    assert_eq!(events[0]["browserName"], "firefox");
    assert_eq!(events[0]["title"], "ff trace");
    let action = |method: &str| {
        events
            .iter()
            .find(|e| e["type"] == "before" && e["method"] == method)
            .unwrap_or_else(|| panic!("no {method} action in {events:#?}"))
            .clone()
    };
    let click = action("click");
    assert_eq!(click["class"], "Frame");
    assert_eq!(click["params"]["selector"], "button");
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "after" && e["callId"] == click["callId"])
    );
    action("setContent");
    assert_eq!(action("goto")["params"]["url"], server.url("/page"));
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "console" && e["text"] == "1"),
        "{events:#?}"
    );
    let frames: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "screencast-frame")
        .collect();
    assert!(!frames.is_empty(), "no screencast frames");
    let jpeg = crate::tracing::file(&files, frames[0]["file"].as_str().unwrap());
    assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    let network: Vec<Value> = String::from_utf8(crate::tracing::file(&files, "trace.network"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(
        network.iter().any(|e| e["type"] == "resource-snapshot"
            && e["snapshot"]["request"]["url"] == server.url("/page")),
        "{network:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn firefox_popups_workers_and_websockets() {
    use chromiumoxide::firefox::PageEvent;
    use chromiumoxide::{WebSocketEvent, WsPayload};
    use futures::StreamExt;
    use std::time::Duration;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/", "text/html", "<p>ws</p>", 0);
    server.route("/one.html", "text/html", "<title>one</title><p>one</p>", 0);
    let port = crate::workers::ws_server().await;
    let browser = Firefox::launch(options).await.unwrap();
    let context = browser.new_context(Default::default()).await.unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();

    // WebSockets: frames both ways, then close.
    let mut events = page.events();
    page.evaluate::<()>(&format!(
        "void (() => {{ const ws = new WebSocket('ws://127.0.0.1:{port}/sock'); ws.onopen = () => ws.send('hello'); \
         let n = 0; ws.onmessage = () => {{ if (++n === 2) ws.close(); }}; }})()"
    ))
    .await
    .unwrap();
    let ws = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(PageEvent::WebSocket(ws)) = events.next().await {
                return ws;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(ws.url(), format!("ws://127.0.0.1:{port}/sock"));
    let mut ws_events = ws.events();
    let mut log = Vec::new();
    while let Some(e) = tokio::time::timeout(Duration::from_secs(10), ws_events.next())
        .await
        .unwrap()
    {
        match e {
            WebSocketEvent::FrameSent(WsPayload::Text(t)) => log.push(format!("sent {t}")),
            WebSocketEvent::FrameReceived(WsPayload::Text(t)) => log.push(format!("recv {t}")),
            WebSocketEvent::FrameReceived(WsPayload::Binary(b)) => log.push(format!("recv {b:?}")),
            WebSocketEvent::Close => {
                log.push("close".into());
                break;
            }
            other => log.push(format!("{other:?}")),
        }
    }
    assert_eq!(
        log,
        ["sent hello", "recv echo:hello", "recv [1, 2, 3]", "close"]
    );
    assert!(ws.is_closed());

    // Workers. Firefox 159's BiDi reports worker realms but only runs code
    // in window realms (`script.evaluate` looks a realm target up among
    // window realms only), so worker.evaluate is a clear error; the
    // worker's console output reaches the page like in Playwright.
    page.evaluate::<()>(
        "void (window.w = new Worker(URL.createObjectURL(new Blob(['self.x = 7; console.log(\"from worker\")'], {type: 'application/javascript'}))))",
    )
    .await
    .unwrap();
    let worker = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(PageEvent::Worker(w)) = events.next().await {
                return w;
            }
        }
    })
    .await
    .unwrap();
    assert!(worker.url().starts_with("blob:"), "{}", worker.url());
    assert_eq!(page.workers().len(), 1);
    let text = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(PageEvent::Console(m)) = events.next().await {
                return m.text().to_owned();
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(text, "from worker");
    let err = worker
        .evaluate::<i32>("1 + 2")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("not supported on Firefox"), "{err}");
    page.evaluate::<()>("window.w.terminate()").await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), worker.wait_for_close())
        .await
        .unwrap();
    assert!(worker.is_closed());
    assert!(page.workers().is_empty());

    // Pop-ups: a target=_blank link and window.open.
    page.set_content(&format!(
        "<a target=_blank href='{}'>open</a>",
        server.url("/one.html")
    ))
    .await
    .unwrap();
    page.locator("a").click().await.unwrap();
    let popup = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(PageEvent::Popup(p)) = events.next().await {
                return p;
            }
        }
    })
    .await
    .unwrap();
    popup
        .wait_for_function(
            "location.href.endsWith('/one.html') && document.readyState === 'complete'",
        )
        .await
        .unwrap();
    assert_eq!(popup.url(), server.url("/one.html"));
    assert_eq!(popup.title().await.unwrap(), "one");
    assert_eq!(context.pages().len(), 2);
    page.evaluate::<()>("void window.open('about:blank')")
        .await
        .unwrap();
    let second = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(PageEvent::Popup(p)) = events.next().await {
                return p;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(second.evaluate::<i32>("1 + 1").await.unwrap(), 2);
    popup.close().await.unwrap();
    second.close().await.unwrap();
    assert_eq!(context.pages().len(), 1);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn firefox_persistent_context_and_connect() {
    use chromiumoxide::ContextOptions;
    use std::io::BufRead;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route(
        "/set.html",
        "text/html",
        "<script>localStorage.k = 'v'; document.cookie = 'c=1; max-age=3600'</script>",
        0,
    );
    server.route("/get.html", "text/html", "<p>get</p>", 0);

    // A persistent profile keeps storage across launches; the context has
    // the window Firefox opened.
    let dir = std::env::temp_dir().join(format!("cox-ff-persistent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let context =
        Firefox::launch_persistent_context(&dir, options.clone(), ContextOptions::default())
            .await
            .unwrap();
    assert_eq!(context.pages().len(), 1);
    let page = context.pages().remove(0);
    page.goto(&server.url("/set.html")).await.unwrap();
    page.wait_for_function("localStorage.k === 'v'")
        .await
        .unwrap();
    context.close().await.unwrap();
    assert!(dir.join("prefs.js").exists(), "the profile stays");

    let context =
        Firefox::launch_persistent_context(&dir, options.clone(), ContextOptions::default())
            .await
            .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/get.html")).await.unwrap();
    assert_eq!(
        page.evaluate::<Option<String>>("localStorage.k")
            .await
            .unwrap(),
        Some("v".into())
    );
    let cookies = context.cookies().await.unwrap();
    assert!(
        cookies.iter().any(|c| c.name == "c" && c.value == "1"),
        "{cookies:?}"
    );
    context.close().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    // Connecting to a Firefox started elsewhere; close only disconnects.
    let profile = std::env::temp_dir().join(format!("cox-ff-connect-{}", std::process::id()));
    std::fs::create_dir_all(&profile).unwrap();
    let mut child = std::process::Command::new(options.executable_path.as_ref().unwrap())
        .args(["-no-remote", "-headless", "-profile"])
        .arg(&profile)
        .args(["--remote-debugging-port=0", "about:blank"])
        .env("MOZ_DISABLE_SAFE_MODE_KEY", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = std::io::BufReader::new(child.stderr.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in stderr.lines().map_while(Result::ok) {
            if let Some(url) = line.split("WebDriver BiDi listening on ").nth(1) {
                let _ = tx.send(url.trim().to_owned());
            }
        }
    });
    let endpoint = rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap();
    let browser = Firefox::connect(&endpoint).await.unwrap();
    assert!(!browser.version().await.unwrap().is_empty());
    let page = browser.new_page().await.unwrap();
    page.set_content("<p>connected</p>").await.unwrap();
    assert_eq!(page.evaluate::<i32>("1 + 1").await.unwrap(), 2);
    browser.close().await.unwrap();
    assert!(child.try_wait().unwrap().is_none(), "Firefox keeps running");
    // The session ended, so another client can connect.
    let again = Firefox::connect(&endpoint).await.unwrap();
    again.close().await.unwrap();
    child.kill().unwrap();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&profile);
}

#[tokio::test]
async fn firefox_trace_snapshots_chunks_and_groups() {
    use chromiumoxide::TracingStart;
    use serde_json::Value;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route(
        "/p.html",
        "text/html",
        "<link rel=stylesheet href=\"/s.css\"><button>B</button>",
        0,
    );
    server.route("/s.css", "text/css", "button { color: red; }", 0);
    server.route("/two", "text/html", "<p>two</p>", 0);
    let browser = Firefox::launch(options).await.unwrap();
    let context = browser.new_context(Default::default()).await.unwrap();
    let tracing = context.tracing();
    tracing
        .start(TracingStart {
            snapshots: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    tracing.group("first group").await.unwrap();
    page.goto(&server.url("/p.html")).await.unwrap();
    page.locator("button").click().await.unwrap();
    tracing.group_end().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let dir = std::env::temp_dir().join(format!("cox-ff-snap-{}", std::process::id()));
    let first = dir.join("one.zip");
    tracing.stop_chunk(Some(&first)).await.unwrap();
    tracing.start_chunk(Some("second")).await.unwrap();
    page.goto(&server.url("/two")).await.unwrap();
    let second = dir.join("two.zip");
    tracing.stop_chunk(Some(&second)).await.unwrap();
    tracing.stop(None).await.unwrap();

    let files = crate::tracing::unzip(&first);
    let lines = |name: &str| -> Vec<Value> {
        String::from_utf8(crate::tracing::file(&files, name))
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    };
    // The stylesheet is stored once, under its hash, as in Playwright.
    let css = "resources/3fe33742618fac5890c933e5782ff6af7a2f181a.css";
    assert_eq!(crate::tracing::file(&files, css), b"button { color: red; }");
    let network = lines("trace.network");
    let entry = network
        .iter()
        .find(|e| e["snapshot"]["request"]["url"] == server.url("/s.css"))
        .expect("stylesheet entry");
    assert_eq!(entry["snapshot"]["response"]["content"]["_file"], css);

    let events = lines("trace.trace");
    let group = events
        .iter()
        .find(|e| e["type"] == "before" && e["method"] == "tracingGroup")
        .expect("group");
    assert_eq!(group["params"]["name"], "first group");
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "after" && e["callId"] == group["callId"])
    );
    let call = |method: &str| {
        events
            .iter()
            .find(|e| e["type"] == "before" && e["method"] == method)
            .unwrap_or_else(|| panic!("no {method}"))
            .clone()
    };
    let snaps = |call_id: &Value| -> Vec<Value> {
        events
            .iter()
            .filter(|e| e["type"] == "frame-snapshot" && e["snapshot"]["callId"] == *call_id)
            .map(|e| e["snapshot"].clone())
            .collect()
    };
    let click = snaps(&call("click")["callId"]);
    let phases: Vec<&str> = click.iter().map(|s| s["phase"].as_str().unwrap()).collect();
    assert_eq!(phases, ["before", "after"]);
    let after = &click[1];
    assert_eq!(after["frameUrl"], server.url("/p.html"));
    assert_eq!(after["isMainFrame"], true);
    assert_eq!(
        after["viewport"],
        serde_json::json!({"width": 1280, "height": 720})
    );
    assert!(after["frameId"].as_str().unwrap().starts_with("frame@"));
    let goto_after = snaps(&call("goto")["callId"])
        .into_iter()
        .find(|s| s["phase"] == "after")
        .expect("goto snapshot");
    assert_eq!(
        goto_after["html"][3],
        serde_json::json!(["BODY", {}, ["BUTTON", {}, "B"]]),
        "{}",
        goto_after["html"]
    );

    let two: Vec<Value> = String::from_utf8(crate::tracing::file(
        &crate::tracing::unzip(&second),
        "trace.trace",
    ))
    .unwrap()
    .lines()
    .map(|l| serde_json::from_str(l).unwrap())
    .collect();
    assert_eq!(two[0]["type"], "context-options");
    assert_eq!(two[0]["title"], "second");
    let gotos: Vec<&str> = two
        .iter()
        .filter(|e| e["type"] == "before" && e["method"] == "goto")
        .map(|e| e["params"]["url"].as_str().unwrap())
        .collect();
    assert_eq!(gotos, [server.url("/two")]);
    let _ = std::fs::remove_dir_all(&dir);
    browser.close().await.unwrap();
}

/// Firefox waits (waitForURL, waitForLoadState, waitForRequest,
/// waitForResponse) and locator extras (filter, and, or, all, boundingBox,
/// evaluate).
#[tokio::test]
async fn firefox_waits_and_locator_extras() {
    use chromiumoxide::{Filter, WaitUntil};
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/a.html", "text/html", "<a href='/b.html'>next</a>", 0);
    server.route("/b.html", "text/html", "<title>b</title>", 0);
    server.route("/data.txt", "text/plain", "data", 0);
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/a.html")).await.unwrap();

    let clicked = {
        let page = page.clone();
        tokio::spawn(async move { page.locator("a").click().await })
    };
    page.wait_for_url("**/b.html").await.unwrap();
    clicked.await.unwrap().unwrap();
    page.wait_for_load_state(WaitUntil::Load).await.unwrap();
    page.wait_for_load_state(WaitUntil::DomContentLoaded)
        .await
        .unwrap();
    page.wait_for_load_state(WaitUntil::NetworkIdle)
        .await
        .unwrap();
    assert_eq!(page.title().await.unwrap(), "b");

    let request = page.wait_for_request("**/data.txt");
    let response = page.wait_for_response("**/data.txt");
    page.evaluate::<String>("fetch('/data.txt').then(r => r.text())")
        .await
        .unwrap();
    assert_eq!(request.await.unwrap().method(), "GET");
    assert_eq!(response.await.unwrap().status(), 200);

    page.set_content("<li>apple</li><li class=x>banana</li><li>cherry</li><button>b</button>")
        .await
        .unwrap();
    let li = page.locator("li");
    assert_eq!(
        li.filter(Filter {
            has_text: Some("an".into()),
            ..Default::default()
        })
        .inner_text()
        .await
        .unwrap(),
        "banana"
    );
    assert_eq!(li.and(&page.locator(".x")).count().await.unwrap(), 1);
    assert_eq!(
        page.locator("li.x")
            .or(&page.locator("button"))
            .count()
            .await
            .unwrap(),
        2
    );
    let all = li.all().await.unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[2].inner_text().await.unwrap(), "cherry");
    let b = page.locator(".x").bounding_box().await.unwrap().unwrap();
    assert!(b.width > 0.0 && b.height > 0.0, "{b:?}");
    assert_eq!(
        page.locator(".x")
            .evaluate::<String>("e => e.className + ':' + e.textContent")
            .await
            .unwrap(),
        "x:banana"
    );
    browser.close().await.unwrap();
}

/// Firefox: the baseURL context option and locator.screenshot().
#[tokio::test]
async fn firefox_base_url_and_locator_screenshot() {
    use chromiumoxide::ContextOptions;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/b.html", "text/html", "<title>based</title>", 0);
    let browser = Firefox::launch(options).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            base_url: Some(server.url("/")),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto("/b.html").await.unwrap();
    assert_eq!(page.title().await.unwrap(), "based");

    page.set_content(
        "<body style='margin:0'><div style='height:300px'></div>\
         <div id=box style='width:50px;height:30px;margin-left:20px;background:rgb(255,0,0)'></div></body>",
    )
    .await
    .unwrap();
    let png = page.locator("#box").screenshot().await.unwrap();
    let (w, h, rgba) = crate::screenshot::decode(&png);
    assert_eq!((w, h), (50, 30));
    assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
    browser.close().await.unwrap();
}

/// Firefox: the rest of Playwright's Locator (states, blur, clear,
/// dispatchEvent, evaluateAll, selectText, setChecked, scrollIntoView,
/// describe, normalize, visible, within, contentFrame, drop, tap).
#[tokio::test]
async fn firefox_locator_helpers() {
    use chromiumoxide::DropPayload;
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
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

    // tap needs a touch context, which BiDi cannot create.
    let err = page.locator("#p").tap().await.unwrap_err().to_string();
    assert!(err.contains("hasTouch"), "{err}");
    browser.close().await.unwrap();
}

/// Firefox: Frame objects (main frame, child frames, parent, name, goto,
/// content, locators, waits, script and style tags) and Playwright's
/// selector shortcuts on Page and Frame (`page.click(selector)`, ...).
#[tokio::test]
async fn firefox_frames_and_selector_shortcuts() {
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
    let browser = Firefox::launch(options).await.unwrap();
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

/// Firefox: the rest of Page (recent console messages, errors and
/// requests, default timeouts, opener, context, bringToFront,
/// waitForNavigation, pickLocator, pause, and the methods BiDi cannot do)
/// and FrameLocator's nth/first/last/owner.
#[tokio::test]
async fn firefox_page_details_and_frame_locator_nth() {
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/a.html", "text/html", "<a href='/b.html'>b</a>", 0);
    server.route("/b.html", "text/html", "<title>b</title>", 0);
    let browser = Firefox::launch(options).await.unwrap();
    let context = browser.new_context(Default::default()).await.unwrap();
    let page = context.new_page().await.unwrap();
    assert!(page.context().is_some());
    assert!(page.opener().is_none());
    page.bring_to_front().await.unwrap();

    // Recent console messages, page errors and requests.
    page.goto(&server.url("/a.html")).await.unwrap();
    page.evaluate::<serde_json::Value>(
        "console.log('one'); setTimeout(() => { throw new Error('boom'); })",
    )
    .await
    .unwrap();
    page.wait_for_function("true").await.unwrap();
    page.wait_for_timeout(std::time::Duration::from_millis(200))
        .await;
    assert!(page.console_messages().iter().any(|m| m.text() == "one"));
    assert!(
        page.page_errors()
            .iter()
            .any(|e| e.message().contains("boom"))
    );
    assert!(page.requests().iter().any(|r| r.url().ends_with("/a.html")));
    page.clear_console_messages();
    page.clear_page_errors();
    assert!(page.console_messages().is_empty());
    assert!(page.page_errors().is_empty());

    // waitForNavigation: listens from the call on.
    let nav = page.wait_for_navigation();
    page.locator("a").click().await.unwrap();
    nav.await.unwrap();
    assert_eq!(page.title().await.unwrap(), "b");

    // Default timeouts apply to locator actions.
    page.set_default_timeout(std::time::Duration::from_millis(300));
    let started = std::time::Instant::now();
    let err = page.locator("#missing").click().await.unwrap_err();
    assert!(
        matches!(err, chromiumoxide::Error::Timeout { ms: 300, .. }),
        "{err}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    page.set_default_navigation_timeout(std::time::Duration::from_secs(30));
    context.set_default_timeout(std::time::Duration::from_secs(30));
    context.set_default_navigation_timeout(std::time::Duration::from_secs(30));

    // Opener of a pop-up.
    page.set_content("<a target=_blank href='about:blank'>pop</a>")
        .await
        .unwrap();
    let mut events = page.events();
    page.locator("a").click().await.unwrap();
    let popup = loop {
        use futures::StreamExt;
        if let Some(chromiumoxide::firefox::PageEvent::Popup(p)) = events.next().await {
            break p;
        }
    };
    assert!(popup.opener().is_some());

    // FrameLocator nth / first / last / owner, get_by_alt_text / title.
    page.set_content(
        "<iframe srcdoc=\"<img alt=A title=T src='data:,'>\"></iframe><iframe srcdoc='<p>2</p>'></iframe>",
    )
    .await
    .unwrap();
    let frames = page.frame_locator("iframe");
    assert_eq!(
        frames
            .first()
            .get_by_alt_text("A", true)
            .count()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        frames
            .first()
            .get_by_title("T", true)
            .count()
            .await
            .unwrap(),
        1
    );
    assert_eq!(frames.last().locator("p").inner_text().await.unwrap(), "2");
    assert_eq!(frames.nth(1).locator("p").count().await.unwrap(), 1);
    assert_eq!(frames.nth(1).owner().count().await.unwrap(), 1);

    // pickLocator: a click in the page picks the element; cancel stops it.
    page.set_content("<button id=pick>pick me</button>")
        .await
        .unwrap();
    let picking = {
        let page = page.clone();
        tokio::spawn(async move { page.pick_locator().await })
    };
    page.wait_for_timeout(std::time::Duration::from_millis(300))
        .await;
    page.mouse().click(10.0, 10.0).await.unwrap();
    let picked = picking.await.unwrap().unwrap();
    assert_eq!(picked.inner_text().await.unwrap(), "pick me");
    let cancelled = {
        let page = page.clone();
        tokio::spawn(async move { page.pick_locator().await })
    };
    page.wait_for_timeout(std::time::Duration::from_millis(200))
        .await;
    page.cancel_pick_locator().await.unwrap();
    assert!(cancelled.await.unwrap().is_err());

    // pause() waits for context.resume().
    let paused = {
        let page = page.clone();
        tokio::spawn(async move { page.pause().await })
    };
    page.wait_for_timeout(std::time::Duration::from_millis(100))
        .await;
    assert!(!paused.is_finished());
    context.resume();
    paused.await.unwrap().unwrap();

    // What WebDriver BiDi cannot do in Firefox: an error that says so.
    for err in [
        page.request_gc().await.unwrap_err().to_string(),
        page.emulate_media(Default::default())
            .await
            .unwrap_err()
            .to_string(),
    ] {
        assert!(err.contains("BiDi"), "{err}");
    }
    browser.close().await.unwrap();
}

/// Firefox: context settings after creation (extra headers, offline,
/// geolocation, permissions, HTTP credentials, storage state), context
/// events, and the rest of Browser and BrowserContext.
#[tokio::test]
async fn firefox_context_settings_and_events() {
    use chromiumoxide::firefox::ContextEvent;
    use chromiumoxide::{ContextOptions, Geolocation, HttpCredentials, StorageState};
    use futures::StreamExt;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/p.html", "text/html", "<title>p</title>", 0);
    let browser = Firefox::launch(options).await.unwrap();
    assert_eq!(browser.browser_type().name(), "firefox");
    assert!(browser.is_connected());
    let context = browser
        .new_context(ContextOptions {
            permissions: vec!["geolocation".into()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(browser.contexts().len(), 1);
    assert!(context.browser().is_some());
    assert!(!context.is_closed());
    assert!(context.background_pages().is_empty());
    assert!(context.service_workers().is_empty());

    // Context events: a new page, then its console messages.
    let mut events = context.events();
    let page = context.new_page().await.unwrap();
    assert!(matches!(events.next().await, Some(ContextEvent::Page(_))));
    page.evaluate::<()>("console.log('from page')")
        .await
        .unwrap();
    let console = context.wait_for_event(|e| match e {
        ContextEvent::Console(m) => Some(m.text().to_owned()),
        _ => None,
    });
    page.evaluate::<()>("console.log('waited')").await.unwrap();
    assert_eq!(console.await.unwrap(), "waited");

    // Extra headers on the context and the page.
    context
        .set_extra_http_headers(&[("x-ctx", "1")])
        .await
        .unwrap();
    page.set_extra_http_headers(&[("x-page", "2")])
        .await
        .unwrap();
    page.goto(&server.url("/dump")).await.unwrap();
    let echoed: serde_json::Value = page
        .evaluate("JSON.parse(document.body.innerText)")
        .await
        .unwrap();
    assert_eq!(echoed["headers"]["x-ctx"], "1", "{echoed}");
    assert_eq!(echoed["headers"]["x-page"], "2", "{echoed}");

    // Geolocation and permissions.
    page.goto(&server.url("/p.html")).await.unwrap();
    context
        .set_geolocation(Some(Geolocation {
            latitude: 10.0,
            longitude: 20.0,
            accuracy: 1.0,
        }))
        .await
        .unwrap();
    let state: String = page
        .evaluate("navigator.permissions.query({ name: 'geolocation' }).then(s => s.state)")
        .await
        .unwrap();
    assert_eq!(state, "granted");
    let lat: f64 = page
        .evaluate(
            "new Promise((ok, err) => navigator.geolocation.getCurrentPosition(p => ok(p.coords.latitude), err))",
        )
        .await
        .unwrap();
    assert_eq!(lat, 10.0);
    context.clear_permissions().await.unwrap();
    let state: String = page
        .evaluate("navigator.permissions.query({ name: 'geolocation' }).then(s => s.state)")
        .await
        .unwrap();
    assert_eq!(state, "prompt");
    context
        .grant_permissions(
            &["geolocation"],
            Some(server.url("/").trim_end_matches('/')),
        )
        .await
        .unwrap();
    let state: String = page
        .evaluate("navigator.permissions.query({ name: 'geolocation' }).then(s => s.state)")
        .await
        .unwrap();
    assert_eq!(state, "granted");

    // Offline.
    context.set_offline(true).await.unwrap();
    assert!(page.goto(&server.url("/p.html")).await.is_err());
    assert!(!page.evaluate::<bool>("navigator.onLine").await.unwrap());
    context.set_offline(false).await.unwrap();
    page.goto(&server.url("/p.html")).await.unwrap();

    // HTTP credentials set later.
    context
        .set_http_credentials(Some(HttpCredentials {
            username: "user".into(),
            password: "pass".into(),
            origin: None,
        }))
        .await
        .unwrap();
    page.goto(&server.url("/protected")).await.unwrap();
    assert_eq!(page.inner_text("body").await.unwrap(), "secret ok");

    // setStorageState replaces cookies and localStorage.
    page.goto(&server.url("/p.html")).await.unwrap();
    page.evaluate::<()>("document.cookie = 'a=1'; localStorage.setItem('k', 'v')")
        .await
        .unwrap();
    let saved = context.storage_state().await.unwrap();
    context
        .set_storage_state(&StorageState::default())
        .await
        .unwrap();
    assert!(context.cookies().await.unwrap().is_empty());
    context.set_storage_state(&saved).await.unwrap();
    // `a=1`, and the `from_server` cookie /dump set earlier.
    assert_eq!(context.cookies().await.unwrap().len(), saved.cookies.len());
    assert!(
        context
            .cookies()
            .await
            .unwrap()
            .iter()
            .any(|c| c.name == "a")
    );
    page.reload().await.unwrap();
    assert_eq!(
        page.evaluate::<String>("localStorage.getItem('k')")
            .await
            .unwrap(),
        "v"
    );

    // Chromium-only parts: an error that says so.
    for err in [
        context
            .new_cdp_session(&page)
            .await
            .unwrap_err()
            .to_string(),
        browser
            .new_browser_cdp_session()
            .await
            .unwrap_err()
            .to_string(),
        browser
            .start_tracing(None, &[])
            .await
            .unwrap_err()
            .to_string(),
        browser.stop_tracing().await.unwrap_err().to_string(),
    ] {
        assert!(err.contains("Chromium"), "{err}");
    }
    context.close().await.unwrap();
    assert!(context.is_closed());
    browser.close().await.unwrap();
    assert!(!browser.is_connected());
}

/// Firefox: JSHandle and ElementHandle (evaluateHandle, elementHandle(s),
/// waitForSelector, frameElement, contentFrame, ownerFrame and the element
/// actions), over BiDi remote references.
#[tokio::test]
async fn firefox_js_and_element_handles() {
    use chromiumoxide::WaitForState;
    let Some(options) = options() else {
        return;
    };
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<input id=t value=v><button id=b onclick=\"this.textContent='hit'\">b</button>\
         <input id=c type=checkbox><select id=s><option>x</option><option>y</option></select>\
         <ul><li>1</li><li>2</li></ul><iframe name=f srcdoc='<p>in</p>'></iframe>",
    )
    .await
    .unwrap();
    page.wait_for_function("frames[0] && frames[0].document.readyState === 'complete'")
        .await
        .unwrap();

    // JSHandle: values, properties, evaluate, dispose.
    let obj = page
        .evaluate_handle("({ a: 1, b: { c: 'x' } })")
        .await
        .unwrap();
    assert_eq!(
        obj.json_value().await.unwrap(),
        serde_json::json!({"a": 1, "b": {"c": "x"}})
    );
    let b = obj.get_property("b").await.unwrap();
    assert_eq!(b.json_value().await.unwrap(), serde_json::json!({"c": "x"}));
    let props = obj.get_properties().await.unwrap();
    assert_eq!(props.len(), 2);
    assert_eq!(obj.evaluate::<i64>("o => o.a + 1").await.unwrap(), 2);
    let c = b.evaluate_handle("o => o.c").await.unwrap();
    assert_eq!(c.json_value().await.unwrap(), "x");
    assert!(obj.as_element().is_none());
    obj.dispose().await.unwrap();

    // ElementHandle from evaluateHandle, a locator and waitForSelector.
    let doc_button = page
        .evaluate_handle("document.getElementById('b')")
        .await
        .unwrap();
    let button = doc_button.as_element().expect("an element");
    button.click().await.unwrap();
    assert_eq!(button.inner_text().await.unwrap(), "hit");
    assert_eq!(button.text_content().await.unwrap().as_deref(), Some("hit"));
    assert_eq!(button.inner_html().await.unwrap(), "hit");
    assert_eq!(
        button.get_attribute("id").await.unwrap().as_deref(),
        Some("b")
    );
    assert!(button.is_visible().await.unwrap() && !button.is_hidden().await.unwrap());
    assert!(button.is_enabled().await.unwrap() && !button.is_disabled().await.unwrap());
    assert!(button.bounding_box().await.unwrap().is_some());
    assert_eq!(&button.screenshot().await.unwrap()[1..4], b"PNG");
    button.hover().await.unwrap();
    button.dblclick().await.unwrap();
    button.dispatch_event("click").await.unwrap();
    button.scroll_into_view_if_needed().await.unwrap();
    button.focus().await.unwrap();
    button.wait_for_element_state("visible").await.unwrap();
    assert_eq!(button.evaluate::<String>("e => e.id").await.unwrap(), "b");

    let input = page.locator("#t").element_handle().await.unwrap();
    assert!(input.is_editable().await.unwrap());
    input.fill("new").await.unwrap();
    assert_eq!(input.input_value().await.unwrap(), "new");
    input.select_text().await.unwrap();
    input.press("End").await.unwrap();
    let check = page
        .wait_for_selector("#c", WaitForState::Visible)
        .await
        .unwrap()
        .unwrap();
    check.check().await.unwrap();
    assert!(check.is_checked().await.unwrap());
    check.uncheck().await.unwrap();
    check.set_checked(true).await.unwrap();
    let select = page.locator("#s").element_handle().await.unwrap();
    assert_eq!(select.select_option(&["y"]).await.unwrap(), ["y"]);
    let ul = page.locator("ul").element_handle().await.unwrap();
    let li = ul
        .wait_for_selector("li >> nth=1", WaitForState::Attached)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(li.inner_text().await.unwrap(), "2");
    assert_eq!(page.locator("li").element_handles().await.unwrap().len(), 2);
    assert!(
        page.wait_for_selector("#gone", WaitForState::Detached)
            .await
            .unwrap()
            .is_none()
    );

    // Frames: frameElement, contentFrame, ownerFrame; frame and locator
    // evaluateHandle.
    let frame = page.frame("f").await.unwrap().unwrap();
    let iframe = frame.frame_element().await.unwrap();
    assert_eq!(
        iframe.get_attribute("name").await.unwrap().as_deref(),
        Some("f")
    );
    let content = iframe.content_frame().await.unwrap().unwrap();
    assert_eq!(
        content
            .evaluate::<String>("document.body.textContent")
            .await
            .unwrap(),
        "in"
    );
    let p = frame
        .wait_for_selector("p", WaitForState::Visible)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        p.owner_frame()
            .await
            .unwrap()
            .unwrap()
            .evaluate::<String>("name")
            .await
            .unwrap(),
        "f"
    );
    let h = frame.evaluate_handle("document.title").await.unwrap();
    assert_eq!(h.json_value().await.unwrap(), "");
    let lh = page
        .locator("#b")
        .evaluate_handle("e => e.id")
        .await
        .unwrap();
    assert_eq!(lh.json_value().await.unwrap(), "b");

    // tap needs hasTouch, which BiDi cannot set in Firefox.
    assert!(
        button
            .tap()
            .await
            .unwrap_err()
            .to_string()
            .contains("hasTouch")
    );
    browser.close().await.unwrap();
}

/// Firefox: Request and Response details (post data, resource type,
/// navigation, frame, redirects, response() / finished(), bodies, headers,
/// timing, sizes, HTTP version).
#[tokio::test]
async fn firefox_request_and_response_details() {
    use chromiumoxide::firefox::PageEvent;
    use futures::StreamExt;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/page.html", "text/html", "<title>p</title>", 0);
    server.route("/data.json", "application/json", r#"{"a":1}"#, 0);
    server.respond("/redirect", "302 Found", &[("Location", "/page.html")], "");
    let browser = Firefox::launch(options).await.unwrap();
    let page = browser.new_page().await.unwrap();

    // A navigation through a redirect.
    let mut events = page.events();
    page.goto(&server.url("/redirect")).await.unwrap();
    let mut requests = Vec::new();
    let mut responses = Vec::new();
    while responses.len() < 2 {
        match events.next().await.unwrap() {
            PageEvent::Request(r) => requests.push(r),
            PageEvent::Response(r) => responses.push(r),
            _ => {}
        }
    }
    let final_request = requests.last().unwrap().clone();
    assert!(final_request.url().ends_with("/page.html"));
    assert!(final_request.is_navigation_request());
    assert_eq!(final_request.resource_type(), "document");
    let first = final_request.redirected_from().expect("redirected from");
    assert!(first.url().ends_with("/redirect"));
    assert!(first.redirected_to().unwrap().url().ends_with("/page.html"));
    assert_eq!(first.existing_response().unwrap().status(), 302);
    let response = final_request.response().await.unwrap().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "<title>p</title>");
    assert_eq!(response.http_version(), "HTTP/1.1");
    assert!(response.request().url().ends_with("/page.html"));
    assert!(response.frame().await.unwrap().is_some());
    assert!(final_request.frame().await.unwrap().is_some());
    assert!(response.finished().await.unwrap().is_none());
    assert_eq!(
        response
            .all_headers()
            .await
            .unwrap()
            .get("content-type")
            .map(String::as_str),
        Some("text/html")
    );
    assert!(
        response
            .headers_array()
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
    );
    assert_eq!(response.header_values("content-type"), ["text/html"]);
    assert!(!response.from_service_worker());
    assert!(final_request.service_worker().is_none());
    let timing = final_request.timing();
    assert!(
        timing.start_time > 0.0 && timing.response_end >= 0.0,
        "{timing:?}"
    );
    let sizes = final_request.sizes().await.unwrap();
    assert!(sizes.response_body_size > 0, "{sizes:?}");
    assert!(
        final_request
            .all_headers()
            .await
            .unwrap()
            .contains_key("user-agent")
    );
    assert!(!final_request.headers_array().is_empty());

    // A fetch with a body, and its JSON response.
    let request = page.wait_for_request("**/data.json");
    let body: serde_json::Value = page
        .evaluate("fetch('/data.json', { method: 'POST', body: JSON.stringify({ q: 2 }) }).then(r => r.json())")
        .await
        .unwrap();
    assert_eq!(body["a"], 1);
    let request = request.await.unwrap();
    assert_eq!(request.method(), "POST");
    assert_eq!(request.resource_type(), "fetch");
    assert!(!request.is_navigation_request());
    assert_eq!(
        request.post_data().await.unwrap().as_deref(),
        Some(r#"{"q":2}"#)
    );
    assert_eq!(
        request.post_data_buffer().await.unwrap().unwrap(),
        br#"{"q":2}"#
    );
    assert_eq!(
        request
            .post_data_json::<serde_json::Value>()
            .await
            .unwrap()
            .unwrap()["q"],
        2
    );
    let response = request.response().await.unwrap().unwrap();
    assert_eq!(response.json::<serde_json::Value>().await.unwrap()["a"], 1);
    assert_eq!(response.body().await.unwrap(), br#"{"a":1}"#);
    // BiDi does not report the server address or TLS details.
    assert!(response.server_addr().is_none());
    assert!(response.security_details().is_none());
    browser.close().await.unwrap();
}

/// Firefox: the event objects' details (console message args, location,
/// page, timestamp; dialog page; download page, delete, read stream; file
/// chooser), Worker.evaluateHandle / waitForEvent, Frame.waitForNavigation
/// and the touchscreen.
#[tokio::test]
async fn firefox_event_object_details() {
    use chromiumoxide::firefox::{PageEvent, WorkerEvent};
    use futures::{AsyncReadExt, StreamExt};
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.respond(
        "/file.txt",
        "200 OK",
        &[
            ("Content-Type", "text/plain"),
            ("Content-Disposition", "attachment; filename=file.txt"),
        ],
        "downloaded",
    );
    server.route(
        "/w.js",
        "text/javascript",
        "onmessage = e => postMessage(e.data * 2);",
        0,
    );
    server.route("/kid.html", "text/html", "<a href='/kid2.html'>next</a>", 0);
    server.route("/kid2.html", "text/html", "<p>two</p>", 0);
    server.route(
        "/p.html",
        "text/html",
        "<iframe name=k src='/kid.html'></iframe>",
        0,
    );
    let browser = Firefox::launch(options).await.unwrap();
    let context = browser
        .new_context(chromiumoxide::ContextOptions {
            accept_downloads: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/p.html")).await.unwrap();
    let mut events = page.events();

    // Console message details.
    page.evaluate::<()>("console.log('hello', 5, { a: 1 })")
        .await
        .unwrap();
    let message = loop {
        if let Some(PageEvent::Console(m)) = events.next().await {
            break m;
        }
    };
    assert_eq!(message.args().len(), 3);
    assert_eq!(message.args()[1].json_value().await.unwrap(), 5);
    assert_eq!(
        message.args()[2].json_value().await.unwrap(),
        serde_json::json!({"a": 1})
    );
    assert!(message.timestamp() > 0.0);
    assert!(message.page().is_some());
    assert!(message.worker().is_none());
    let _ = message.location();

    // Dialog page.
    let dialog = {
        let page = page.clone();
        tokio::spawn(async move { page.evaluate::<()>("alert('hi')").await })
    };
    let d = loop {
        if let Some(PageEvent::Dialog(d)) = events.next().await {
            break d;
        }
    };
    assert!(std::ptr::eq(d.page(), d.page()));
    d.dismiss().await.unwrap();
    dialog.await.unwrap().unwrap();

    // Download page, read stream, delete; cancel is not in WebDriver BiDi.
    page.set_content(&format!("<a href='{}'>dl</a>", server.url("/file.txt")))
        .await
        .unwrap();
    page.locator("a").click().await.unwrap();
    let download = loop {
        if let Some(PageEvent::Download(d)) = events.next().await {
            break d;
        }
    };
    assert!(download.page().is_some());
    let mut text = String::new();
    download
        .create_read_stream()
        .await
        .unwrap()
        .read_to_string(&mut text)
        .await
        .unwrap();
    assert_eq!(text, "downloaded");
    let path = download.path().await.unwrap();
    download.delete().await.unwrap();
    assert!(!path.exists());
    assert!(
        download
            .cancel()
            .await
            .unwrap_err()
            .to_string()
            .contains("BiDi")
    );

    // File chooser.
    page.set_content("<input type=file multiple>")
        .await
        .unwrap();
    page.locator("input").click().await.unwrap();
    let chooser = loop {
        if let Some(PageEvent::FileChooser(c)) = events.next().await {
            break c;
        }
    };
    assert!(chooser.is_multiple());
    assert!(std::ptr::eq(chooser.page(), chooser.page()));
    let file = std::env::temp_dir().join("cox-ff-chooser.txt");
    std::fs::write(&file, "x").unwrap();
    chooser.set_files(&[file.clone()]).await.unwrap();
    assert_eq!(
        chooser
            .element()
            .evaluate::<String>("e => e.files[0].name")
            .await
            .unwrap(),
        "cox-ff-chooser.txt"
    );

    // Worker handles and events.
    page.goto(&server.url("/p.html")).await.unwrap();
    page.evaluate::<()>("window.w = new Worker('/w.js')")
        .await
        .unwrap();
    page.wait_for_function("true").await.unwrap();
    let worker = loop {
        if let Some(w) = page.workers().into_iter().next() {
            break w;
        }
        page.wait_for_timeout(std::time::Duration::from_millis(50))
            .await;
    };
    let h = worker.evaluate_handle("({ x: 1 })").await.unwrap();
    assert_eq!(h.json_value().await.unwrap(), serde_json::json!({"x": 1}));
    let closed = worker.wait_for_event(|e| matches!(e, WorkerEvent::Close).then_some(()));
    page.evaluate::<()>("w.terminate()").await.unwrap();
    // Firefox reports a terminated worker when its realm goes away.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), closed).await;

    // Frame.waitForNavigation.
    let kid = page.frame("k").await.unwrap().unwrap();
    let nav = kid.wait_for_navigation();
    kid.click("a").await.unwrap();
    nav.await.unwrap();
    assert_eq!(kid.inner_text("p").await.unwrap(), "two");

    // Touchscreen needs hasTouch.
    assert!(
        page.touchscreen()
            .tap(1.0, 1.0)
            .await
            .unwrap_err()
            .to_string()
            .contains("hasTouch")
    );
    browser.close().await.unwrap();
}
