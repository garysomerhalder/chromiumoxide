//! Backlog items (docs/BACKLOG.md): options, methods and events Playwright
//! 1.65.1 has that milestones 1-7 did not cover.

use std::time::Duration;

use chromiumoxide::{Browser, Chromium, LaunchOptions, MouseButton, Page};

async fn page_with(html: &str) -> (Browser, Page) {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(html).await.unwrap();
    (browser, page)
}

/// Records `type@performance.now()` for mouse and key events.
const RECORDER: &str = "<button style='width:200px;height:100px'>B</button><input id=i>\
<script>window.ev = []; for (const t of ['mousedown','mouseup','keydown','keyup']) \
document.addEventListener(t, e => ev.push([t, performance.now(), e.key || '']));</script>";

async fn events(page: &Page) -> Vec<(String, f64, String)> {
    page.evaluate("window.ev.splice(0)").await.unwrap()
}

fn gap(ev: &[(String, f64, String)], a: usize, b: usize) -> f64 {
    ev[b].1 - ev[a].1
}

/// Playwright's `delay`: between mousedown and mouseup (and between the
/// clicks of a dblclick), and between keydown and keyup.
#[tokio::test]
async fn input_delays() {
    let (browser, page) = page_with(RECORDER).await;
    page.mouse()
        .click_with_delay(50.0, 50.0, MouseButton::Left, 1, Duration::from_millis(200))
        .await
        .unwrap();
    let ev = events(&page).await;
    assert_eq!(ev.len(), 2, "{ev:?}");
    assert!(gap(&ev, 0, 1) >= 180.0, "{ev:?}");

    page.locator("button")
        .dblclick()
        .delay(Duration::from_millis(150))
        .await
        .unwrap();
    let ev = events(&page).await;
    let kinds: Vec<&str> = ev.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(kinds, ["mousedown", "mouseup", "mousedown", "mouseup"]);
    assert!(gap(&ev, 0, 1) >= 130.0 && gap(&ev, 1, 2) >= 130.0, "{ev:?}");

    page.locator("#i").focus().await.unwrap();
    page.keyboard()
        .press_with_delay("a", Duration::from_millis(150))
        .await
        .unwrap();
    let ev = events(&page).await;
    assert_eq!(ev.len(), 2, "{ev:?}");
    assert!(gap(&ev, 0, 1) >= 130.0, "{ev:?}");

    page.keyboard()
        .type_with_delay("bc", Duration::from_millis(100))
        .await
        .unwrap();
    let ev = events(&page).await;
    assert_eq!(ev.len(), 4, "{ev:?}");
    assert!(gap(&ev, 0, 1) >= 80.0 && gap(&ev, 2, 3) >= 80.0, "{ev:?}");

    page.locator("#i")
        .press_with_delay("d", Duration::from_millis(120))
        .await
        .unwrap();
    page.locator("#i")
        .press_sequentially_with_delay("e", Duration::from_millis(120))
        .await
        .unwrap();
    let ev = events(&page).await;
    assert_eq!(ev.len(), 4, "{ev:?}");
    assert!(gap(&ev, 0, 1) >= 100.0 && gap(&ev, 2, 3) >= 100.0, "{ev:?}");
    assert_eq!(page.locator("#i").input_value().await.unwrap(), "abcde");
    browser.close().await.unwrap();
}

/// A covered element: the retry reason names the covering element, like
/// Playwright's "<div id=\"cover\"></div> intercepts pointer events".
#[tokio::test]
async fn hit_target_message_names_the_element() {
    let (browser, page) =
        page_with("<button>B</button><div id=cover style='position:fixed;inset:0'></div>").await;
    let err = page
        .locator("button")
        .click()
        .timeout(Duration::from_millis(500))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("<div id=\"cover\"") && err.contains("intercepts pointer events"),
        "{err}"
    );
    assert!(!err.contains("hitTargetDescription"), "{err}");
    browser.close().await.unwrap();
}

/// Collects event names from a stream in the background.
fn collect<S, E>(
    stream: S,
    name: fn(&E) -> Option<String>,
) -> std::sync::Arc<std::sync::Mutex<Vec<String>>>
where
    S: futures::Stream<Item = E> + Send + 'static,
    E: Send + 'static,
{
    use futures::StreamExt;
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let out = seen.clone();
    tokio::spawn(async move {
        let mut stream = Box::pin(stream);
        while let Some(e) = stream.next().await {
            if let Some(n) = name(&e) {
                out.lock().unwrap().push(n);
            }
        }
    });
    seen
}

fn page_event_name(e: &chromiumoxide::PageEvent) -> Option<String> {
    use chromiumoxide::PageEvent::*;
    Some(match e {
        // Like Playwright, a frame has no name yet when it attaches, and is
        // gone from the page once detached.
        FrameAttached(_) => "frameattached".into(),
        FrameNavigated(f) => format!("framenavigated:{}", f.name()),
        FrameDetached(_) => "framedetached".into(),
        Dialog(d) => format!("dialog:{}", d.message()),
        DialogClosed(d) => format!("dialogclosed:{}", d.message()),
        Crash => "crash".into(),
        _ => return None,
    })
}

/// `page.on('frameattached' | 'framenavigated' | 'framedetached' |
/// 'dialogclosed' | 'crash')`.
#[tokio::test]
async fn page_frame_dialog_and_crash_events() {
    let (browser, page) = page_with("<p>x</p>").await;
    let seen = collect(page.events(), page_event_name);
    page.goto("data:text/html,<p>main</p>").await.unwrap();
    page.set_content("<iframe name=child srcdoc='<p>c</p>'></iframe>")
        .await
        .unwrap();
    page.evaluate::<()>("document.querySelector('iframe').remove()")
        .await
        .unwrap();
    page.evaluate::<()>("void setTimeout(() => alert('hi'), 0)")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let got = seen.lock().unwrap().clone();
    for want in [
        "framenavigated:",
        "frameattached",
        "framenavigated:child",
        "framedetached",
        "dialog:hi",
        "dialogclosed:hi",
    ] {
        assert!(got.iter().any(|g| g == want), "{want} not in {got:?}");
    }
    let _ = page.goto("chrome://crash").await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        seen.lock().unwrap().iter().any(|g| g == "crash"),
        "{:?}",
        seen.lock().unwrap()
    );
    browser.close().await.unwrap();
}

/// `context.on('pageload' | 'pageclose' | 'download' | 'dialogclosed' |
/// 'frameattached' | ...)` and `browser.on('context' | 'disconnected')`.
#[tokio::test]
async fn context_and_browser_events() {
    use chromiumoxide::{BrowserEvent, ContextEvent};
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let browser_seen = collect(browser.events(), |e: &BrowserEvent| {
        Some(match e {
            BrowserEvent::Context(_) => "context".into(),
            BrowserEvent::Disconnected => "disconnected".into(),
            _ => return None,
        })
    });
    let context = browser
        .new_context(chromiumoxide::ContextOptions::default())
        .await
        .unwrap();
    let seen = collect(context.events(), |e: &ContextEvent| {
        Some(match e {
            ContextEvent::PageLoad(_) => "pageload".into(),
            ContextEvent::PageClose(_) => "pageclose".into(),
            ContextEvent::Download(_) => "download".into(),
            ContextEvent::DialogClosed(_) => "dialogclosed".into(),
            ContextEvent::FrameAttached(_) => "frameattached".into(),
            ContextEvent::FrameNavigated(_) => "framenavigated".into(),
            ContextEvent::FrameDetached(_) => "framedetached".into(),
            _ => return None,
        })
    });
    let page = context.new_page().await.unwrap();
    page.set_content(
        "<a download=f.txt href='data:text/plain,hi'>d</a><iframe srcdoc='x'></iframe>",
    )
    .await
    .unwrap();
    page.locator("a").click().await.unwrap();
    page.evaluate::<()>("void setTimeout(() => alert('x'), 0)")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    page.close().await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let got = seen.lock().unwrap().clone();
    for want in [
        "pageload",
        "frameattached",
        "framenavigated",
        "download",
        "dialogclosed",
        "pageclose",
    ] {
        assert!(got.iter().any(|g| g == want), "{want} not in {got:?}");
    }
    browser.close().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(*browser_seen.lock().unwrap(), ["context", "disconnected"]);
}

const TWO_BUTTONS: &str =
    "<button onclick='window.c=1'>a</button><button onclick='window.c=2'>b</button>";

/// Playwright 1.65.1 (recorded): selector shortcuts and `$` use the first
/// match unless `strictSelectors`; locators are always strict.
#[tokio::test]
async fn selector_shortcuts_strictness_and_query_selector() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(TWO_BUTTONS).await.unwrap();
    page.click("button").await.unwrap();
    assert_eq!(page.evaluate::<i64>("window.c").await.unwrap(), 1);
    assert_eq!(
        page.text_content("button").await.unwrap().as_deref(),
        Some("a")
    );
    assert!(page.is_visible("button").await.unwrap());
    let first = page.query_selector("button").await.unwrap().unwrap();
    assert_eq!(first.text_content().await.unwrap().as_deref(), Some("a"));
    assert!(page.query_selector("nope").await.unwrap().is_none());
    assert_eq!(page.query_selector_all("button").await.unwrap().len(), 2);
    assert_eq!(
        page.eval_on_selector::<String>("button", "e => e.textContent")
            .await
            .unwrap(),
        "a"
    );
    assert_eq!(
        page.eval_on_selector_all_with::<i64, _>("button", "(es, n) => es.length * n", 10)
            .await
            .unwrap(),
        20
    );
    let err = page
        .eval_on_selector::<String>("nope", "e => e.id")
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Failed to find element matching selector \"nope\""),
        "{err}"
    );
    let body = page.query_selector("body").await.unwrap().unwrap();
    assert_eq!(body.query_selector_all("button").await.unwrap().len(), 2);
    assert_eq!(
        body.eval_on_selector::<String>("button:last-child", "e => e.textContent")
            .await
            .unwrap(),
        "b"
    );
    assert_eq!(
        page.main_frame()
            .eval_on_selector_all::<usize>("button", "es => es.length")
            .await
            .unwrap(),
        2
    );
    let err = page
        .locator("button")
        .click()
        .timeout(Duration::from_millis(500))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("strict mode violation"), "{err}");

    let strict = browser
        .new_context(chromiumoxide::ContextOptions {
            strict_selectors: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = strict.new_page().await.unwrap();
    page.set_content(TWO_BUTTONS).await.unwrap();
    for err in [
        page.click("button").await.unwrap_err(),
        page.text_content("button").await.unwrap_err(),
        page.query_selector("button").await.unwrap_err(),
    ] {
        assert!(err.to_string().contains("strict mode violation"), "{err}");
    }
    browser.close().await.unwrap();
}

/// `page.close({ runBeforeUnload: true })` runs `beforeunload` (a dialog
/// Playwright accepts by default) before the page closes.
#[tokio::test]
async fn close_can_run_before_unload() {
    use futures::StreamExt;
    let (browser, page) = page_with(
        "<button>x</button><script>addEventListener('beforeunload', e => { e.preventDefault(); e.returnValue = ''; })</script>",
    )
    .await;
    page.click("button").await.unwrap();
    let mut events = page.events();
    page.close_with(chromiumoxide::PageCloseOptions {
        run_before_unload: true,
    })
    .await
    .unwrap();
    let mut kinds = Vec::new();
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_secs(5), events.next()).await {
        match e {
            chromiumoxide::PageEvent::Dialog(d) => kinds.push(format!("dialog:{}", d.kind())),
            chromiumoxide::PageEvent::Close => {
                kinds.push("close".into());
                break;
            }
            _ => {}
        }
    }
    assert_eq!(kinds, ["dialog:beforeunload", "close"]);
    assert!(page.is_closed());
    browser.close().await.unwrap();
}

/// A server that resets its first `resets` connections (after reading the
/// request), then answers `ok`.
async fn flaky_server(resets: usize) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    tokio::spawn(async move {
        let mut n = 0;
        while let Ok((mut sock, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf).await;
            n += 1;
            if n <= resets {
                let _ = sock.set_linger(Some(Duration::ZERO));
                drop(sock);
                continue;
            }
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await;
        }
    });
    url
}

/// Playwright's `maxRetries`: ECONNRESET is retried with backoff; then
/// "Failed after N attempt(s)".
#[tokio::test]
async fn request_max_retries() {
    let rq = Chromium::request_context(chromiumoxide::ApiRequestOptions::default())
        .await
        .unwrap();
    let url = flaky_server(2).await;
    let r = rq.get(&url).max_retries(2).await.unwrap();
    assert_eq!(r.text().await.unwrap(), "ok");
    let url = flaky_server(2).await;
    let err = rq.get(&url).max_retries(1).await.unwrap_err().to_string();
    assert!(err.contains("Failed after 2 attempt(s)"), "{err}");
    let url = flaky_server(1).await;
    let err = rq.get(&url).await.unwrap_err().to_string();
    assert!(!err.contains("attempt"), "{err}");

    // route.fetch({ maxRetries }) fetches like Playwright, through the
    // context's request.
    let (browser, page) = page_with("<p>x</p>").await;
    let target = flaky_server(1).await;
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let s = seen.clone();
    page.route("**/flaky", move |route: chromiumoxide::Route| {
        let (s, target) = (s.clone(), target.clone());
        async move {
            let r = route.fetch().url(&target).max_retries(1).await;
            *s.lock().unwrap() = Some(r.as_ref().map(|r| r.status()).map_err(|e| e.to_string()));
            let _ = route.fulfill().body("done").await;
        }
    })
    .await
    .unwrap();
    page.goto("http://flaky.test/flaky").await.unwrap();
    assert_eq!(*seen.lock().unwrap(), Some(Ok(200)));
    browser.close().await.unwrap();
}
