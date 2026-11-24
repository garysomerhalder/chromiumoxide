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

/// WebKit: waitForURL / waitForLoadState / waitForRequest / waitForResponse
/// and the locator extras (filter, and, or, all, boundingBox, evaluate,
/// screenshot), with the baseURL context option.
#[tokio::test]
async fn webkit_waits_and_locator_extras() {
    use chromiumoxide::{ContextOptions, Filter, WaitUntil};
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/a.html", "text/html", "<a href='/b.html'>next</a>", 0);
    server.route("/b.html", "text/html", "<title>b</title>", 0);
    server.route("/data.txt", "text/plain", "data", 0);
    let browser = WebKit::launch(options).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            base_url: Some(server.url("/")),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto("/a.html").await.unwrap();

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

/// WebKit: the rest of Page (context, opener, bringToFront, recent console
/// messages / errors / requests, waitForNavigation, default timeouts,
/// pickLocator, pause, requestGC, emulateMedia) and FrameLocator's nth,
/// first, last and owner.
#[tokio::test]
async fn webkit_page_details_and_frame_locator_nth() {
    use chromiumoxide::{ColorScheme, EmulateMedia, Media};
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/a.html", "text/html", "<a href='/b.html'>b</a>", 0);
    server.route("/b.html", "text/html", "<title>b</title>", 0);
    let browser = WebKit::launch(options).await.unwrap();
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
        if let Some(chromiumoxide::webkit::PageEvent::Popup(p)) = events.next().await {
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

    // requestGC and emulateMedia work in WebKit.
    page.request_gc().await.unwrap();
    page.emulate_media(EmulateMedia {
        media: Some(Media::Print),
        color_scheme: Some(ColorScheme::Dark),
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(
        page.evaluate::<bool>("matchMedia('print').matches")
            .await
            .unwrap()
    );
    assert!(
        page.evaluate::<bool>("matchMedia('(prefers-color-scheme: dark)').matches")
            .await
            .unwrap()
    );
    browser.close().await.unwrap();
}

/// WebKit: context settings after creation (extra headers, offline,
/// geolocation, permissions, HTTP credentials, storage state), context
/// events, and the rest of Browser and BrowserContext.
#[tokio::test]
async fn webkit_context_settings_and_events() {
    use chromiumoxide::webkit::ContextEvent;
    use chromiumoxide::{ContextOptions, Geolocation, HttpCredentials, StorageState};
    use futures::StreamExt;
    let Some(options) = options() else {
        return;
    };
    let server = crate::server::TestServer::start().await;
    server.route("/p.html", "text/html", "<title>p</title>", 0);
    let browser = WebKit::launch(options).await.unwrap();
    assert_eq!(browser.browser_type().name(), "webkit");
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
    assert_eq!(context.pages().len(), 1);
    let console = context.wait_for_event(|e| match e {
        ContextEvent::Console(m) => Some(m.text().to_owned()),
        _ => None,
    });
    page.evaluate::<()>("void console.log('waited')")
        .await
        .unwrap();
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
    let lat: f64 = page
        .evaluate(
            "new Promise((ok, err) => navigator.geolocation.getCurrentPosition(p => ok(p.coords.latitude), err))",
        )
        .await
        .unwrap();
    assert_eq!(lat, 10.0);
    // WebKit keeps a document's geolocation decision until it reloads.
    context.clear_permissions().await.unwrap();
    page.reload().await.unwrap();
    let denied: bool = page
        .evaluate(
            "new Promise(ok => navigator.geolocation.getCurrentPosition(() => ok(false), () => ok(true)))",
        )
        .await
        .unwrap();
    assert!(denied);
    context
        .grant_permissions(
            &["geolocation"],
            Some(server.url("/").trim_end_matches('/')),
        )
        .await
        .unwrap();
    page.reload().await.unwrap();
    let lat: f64 = page
        .evaluate(
            "new Promise((ok, err) => navigator.geolocation.getCurrentPosition(p => ok(p.coords.latitude), err))",
        )
        .await
        .unwrap();
    assert_eq!(lat, 10.0);

    // Offline.
    context.set_offline(true).await.unwrap();
    assert!(page.goto(&server.url("/p.html")).await.is_err());
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
    page.evaluate::<()>("void (document.cookie = 'a=1', localStorage.setItem('k', 'v'))")
        .await
        .unwrap();
    let saved = context.storage_state().await.unwrap();
    context
        .set_storage_state(&StorageState::default())
        .await
        .unwrap();
    assert!(context.cookies().await.unwrap().is_empty());
    context.set_storage_state(&saved).await.unwrap();
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

/// WebKit: JSHandle and ElementHandle (evaluateHandle, elementHandle(s),
/// waitForSelector, frameElement, contentFrame, ownerFrame and the element
/// actions), over BiDi remote references.
#[tokio::test]
async fn webkit_js_and_element_handles() {
    use chromiumoxide::WaitForState;
    let Some(options) = options() else {
        return;
    };
    let browser = WebKit::launch(options).await.unwrap();
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
    let nav = frame.wait_for_navigation();
    frame
        .evaluate::<()>("void setTimeout(() => location.href = 'about:blank#next')")
        .await
        .unwrap();
    nav.await.unwrap();
    let lh = page
        .locator("#b")
        .evaluate_handle("e => e.id")
        .await
        .unwrap();
    assert_eq!(lh.json_value().await.unwrap(), "b");

    // tap needs a hasTouch context.
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
