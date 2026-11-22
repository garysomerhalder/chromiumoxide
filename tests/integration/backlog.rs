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
                // A zero linger resets the connection (RST) without blocking.
                #[allow(deprecated)]
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

/// Playwright's `storageState({ opfs })`: origin private file system entries
/// (`{ path, type, base64 }`), restored into new contexts.
#[tokio::test]
async fn storage_state_with_opfs() {
    let server = crate::server::TestServer::start().await;
    server.route("/", "text/html", "<p>opfs</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(chromiumoxide::ContextOptions::default())
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    page.evaluate::<()>(
        "async () => { const root = await navigator.storage.getDirectory(); \
         const dir = await root.getDirectoryHandle('dir', { create: true }); \
         const f = await dir.getFileHandle('a.txt', { create: true }); \
         const w = await f.createWritable(); await w.write('hello'); await w.close(); }",
    )
    .await
    .unwrap();
    let plain = context.storage_state().await.unwrap();
    assert!(plain.origins.iter().all(|o| o.opfs.is_none()), "{plain:?}");
    let state = context
        .storage_state_with(chromiumoxide::StorageStateOptions {
            opfs: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let opfs = state.origins[0].opfs.clone().expect("opfs entries");
    let file = opfs
        .iter()
        .find(|e| e["path"] == "dir/a.txt")
        .unwrap_or_else(|| panic!("{opfs:?}"));
    assert_eq!(file["type"], "file");
    assert_eq!(file["base64"], "aGVsbG8=");

    let restored = browser
        .new_context(chromiumoxide::ContextOptions {
            storage_state: Some(state.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = restored.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    let text: String = page
        .evaluate(
            "async () => { const root = await navigator.storage.getDirectory(); \
             const dir = await root.getDirectoryHandle('dir'); \
             return await (await (await dir.getFileHandle('a.txt')).getFile()).text(); }",
        )
        .await
        .unwrap();
    assert_eq!(text, "hello");

    // A standalone request context keeps the origins it was given.
    let rq = Chromium::request_context(chromiumoxide::ApiRequestOptions {
        storage_state: Some(state.clone()),
        ..Default::default()
    })
    .await
    .unwrap();
    let back = rq
        .storage_state_with(chromiumoxide::StorageStateOptions {
            opfs: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(back.origins, state.origins);
    let without = rq.storage_state().await.unwrap();
    assert!(without.origins.iter().all(|o| o.opfs.is_none()));
    browser.close().await.unwrap();
}

/// `debugger.runTo(location)`: resume and pause again at the action called
/// from that source line; `pausedDetails().location` names the call site.
#[tokio::test]
async fn debugger_run_to_a_source_location() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser.new_context(Default::default()).await.unwrap();
    let page = ctx.new_page().await.unwrap();
    page.set_content(
        "<script>window.n=[]</script><button id=a onclick=n.push('a')>a</button>\
         <button id=b onclick=n.push('b')>b</button><button id=c onclick=n.push('c')>c</button>",
    )
    .await
    .unwrap();
    let dbg = ctx.debugger();
    let err = dbg
        .run_to(chromiumoxide::SourceLocation::file("backlog.rs"))
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "Debugger is not paused");
    dbg.request_pause().await.unwrap();
    let actions = tokio::spawn({
        let page = page.clone();
        async move {
            let first = line!() + 1;
            page.locator("#a").click().await.unwrap();
            page.locator("#b").click().await.unwrap();
            let third = line!() + 1;
            page.locator("#c").click().await.unwrap();
            (first, third)
        }
    });
    let mut paused = None;
    for _ in 0..500 {
        if let Some(p) = dbg.paused_details() {
            paused = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let p = paused.expect("paused before the first click");
    assert_eq!(p.title, "Click locator('#a')");
    let location = p.location.expect("call site");
    assert!(location.file.ends_with("backlog.rs"), "{location:?}");
    let first = location.line.unwrap();
    dbg.run_to(chromiumoxide::SourceLocation {
        file: "integration/backlog.rs".into(),
        line: Some(first + 3),
        column: None,
    })
    .await
    .unwrap();
    let mut paused = None;
    for _ in 0..500 {
        if let Some(p) = dbg.paused_details() {
            paused = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let p = paused.expect("paused at the third click");
    assert_eq!(p.title, "Click locator('#c')");
    assert_eq!(p.location.unwrap().line, Some(first + 3));
    assert_eq!(page.evaluate::<Vec<String>>("n").await.unwrap(), ["a", "b"]);
    dbg.resume().await.unwrap();
    let (a, c) = actions.await.unwrap();
    assert_eq!((a, c), (first, first + 3));
    assert_eq!(
        page.evaluate::<Vec<String>>("n").await.unwrap(),
        ["a", "b", "c"]
    );
    browser.close().await.unwrap();
}

/// `browserServer.process()`: the browser's process, here its pid.
#[tokio::test]
async fn browser_server_process() {
    let server = Chromium::launch_server(LaunchOptions::default())
        .await
        .unwrap();
    let process = server.process().expect("launched by us");
    let pid = process.pid();
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap();
    let cmdline = String::from_utf8_lossy(&cmdline);
    assert!(cmdline.contains("--remote-debugging-port"), "{cmdline}");
    server.close().await.unwrap();
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}/cmdline")).exists()
            || std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .unwrap_or_default()
                .contains(") Z"),
        "process {pid} ended"
    );
}

/// `download.createReadStream()`: the bytes as an `AsyncRead`.
#[tokio::test]
async fn download_read_stream() {
    use futures::AsyncReadExt;
    let server = crate::server::TestServer::start().await;
    server.respond(
        "/dl",
        "200 OK",
        &[("Content-Disposition", "attachment; filename=f.txt")],
        "streamed bytes",
    );
    let (browser, page) = page_with("").await;
    page.set_content(&format!("<a href='{}'>dl</a>", server.url("/dl")))
        .await
        .unwrap();
    let download = page.wait_for_download().run(page.click("a")).await.unwrap();
    let mut stream = download.create_read_stream().await.unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).await.unwrap();
    assert_eq!(out, "streamed bytes");
    browser.close().await.unwrap();
}

/// Launch options `slowMo`, `downloadsPath`, `artifactsDir` and `tracesDir`.
#[tokio::test]
async fn launch_slow_mo_and_artifact_dirs() {
    let server = crate::server::TestServer::start().await;
    server.respond(
        "/dl",
        "200 OK",
        &[("Content-Disposition", "attachment; filename=f.txt")],
        "abc",
    );
    let root = std::env::temp_dir().join(format!("chromiumoxide-artifacts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    // downloadsPath + slowMo.
    let downloads = root.join("downloads");
    let browser = Chromium::launch(LaunchOptions {
        downloads_path: Some(downloads.clone()),
        slow_mo: Duration::from_millis(150),
        ..Default::default()
    })
    .await
    .unwrap();
    let context = browser
        .new_context(chromiumoxide::ContextOptions {
            accept_downloads: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.set_content(&format!(
        "<a href='{}'>dl</a><button>b</button>",
        server.url("/dl")
    ))
    .await
    .unwrap();
    let start = std::time::Instant::now();
    for _ in 0..3 {
        page.locator("button").click().await.unwrap();
    }
    assert!(
        start.elapsed() >= Duration::from_millis(450),
        "{:?}",
        start.elapsed()
    );
    let start = std::time::Instant::now();
    for _ in 0..3 {
        page.evaluate::<i64>("1").await.unwrap();
    }
    assert!(
        start.elapsed() < Duration::from_millis(400),
        "evaluate is not slowed"
    );
    let download = page.wait_for_download().run(page.click("a")).await.unwrap();
    let path = download.path().await.unwrap();
    assert!(path.starts_with(&downloads), "{path:?}");
    // Downloads go with their context; the directory stays.
    context.close().await.unwrap();
    assert!(!path.exists(), "deleted with its context");
    browser.close().await.unwrap();
    assert!(downloads.is_dir(), "downloadsPath is not removed");

    // artifactsDir (downloads by default) + tracesDir.
    let artifacts = root.join("artifacts");
    let traces = root.join("traces");
    let browser = Chromium::launch(LaunchOptions {
        artifacts_dir: Some(artifacts.clone()),
        traces_dir: Some(traces.clone()),
        ..Default::default()
    })
    .await
    .unwrap();
    let context = browser
        .new_context(chromiumoxide::ContextOptions {
            accept_downloads: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.set_content(&format!("<a href='{}'>dl</a>", server.url("/dl")))
        .await
        .unwrap();
    let download = page.wait_for_download().run(page.click("a")).await.unwrap();
    assert!(download.path().await.unwrap().starts_with(&artifacts));
    context
        .tracing()
        .start(chromiumoxide::TracingStart {
            name: Some("run".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    page.locator("a").hover().await.unwrap();
    context.tracing().stop(None).await.unwrap();
    let trace = std::fs::read_to_string(traces.join("run.trace")).unwrap();
    assert!(trace.contains("\"context-options\""), "{trace}");
    assert!(trace.contains("\"hover\""), "{trace}");
    assert!(traces.join("run.network").is_file());
    browser.close().await.unwrap();
    assert!(artifacts.is_dir(), "artifactsDir is not removed");
    let _ = std::fs::remove_dir_all(&root);
}

/// The child half of the signal tests: launches with the handler for
/// `COX_SIGNAL_CHILD` (`int` or `term`), prints the browser pid, then waits.
#[tokio::test]
#[ignore = "run by the signal tests in a child process"]
async fn signal_child() {
    use futures::StreamExt;
    let Ok(mode) = std::env::var("COX_SIGNAL_CHILD") else {
        return;
    };
    let browser = Chromium::launch(LaunchOptions {
        handle_sigint: mode == "int",
        handle_sigterm: mode == "term",
        ..Default::default()
    })
    .await
    .unwrap();
    let mut events = browser.events();
    println!("BROWSER_PID={}", browser.process_id().unwrap());
    // SIGTERM closes the browser and the program goes on (as in Playwright).
    while let Some(e) = events.next().await {
        if matches!(e, chromiumoxide::BrowserEvent::Disconnected) {
            println!("DISCONNECTED");
            return;
        }
    }
}

/// Runs `signal_child` in `mode`, sends it `signal` once the browser is up;
/// returns its exit status, its output and the browser's profile directory.
fn run_signal_child(mode: &str, signal: &str) -> (std::process::ExitStatus, String, String, u32) {
    use std::io::BufRead;
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "backlog::signal_child",
            "--ignored",
            "--nocapture",
        ])
        .env("COX_SIGNAL_CHILD", mode)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut seen = String::new();
    let pid = loop {
        let mut line = String::new();
        assert!(
            out.read_line(&mut line).unwrap() > 0,
            "child output: {seen}"
        );
        seen.push_str(&line);
        if let Some(pid) = line.trim().strip_prefix("BROWSER_PID=") {
            break pid.parse::<u32>().unwrap();
        }
    };
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap();
    let profile = String::from_utf8_lossy(&cmdline)
        .split('\0')
        .find_map(|a| a.strip_prefix("--user-data-dir=").map(str::to_owned))
        .unwrap();
    let sent = std::process::Command::new("kill")
        .args([signal, &child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut out, &mut rest).unwrap();
    seen.push_str(&rest);
    (child.wait().unwrap(), seen, profile, pid)
}

fn gone(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |s| s.contains(") Z"))
}

/// `handleSIGINT`: Ctrl-C closes the browser, removes its profile, then the
/// program exits with 130.
#[tokio::test]
async fn handle_sigint_closes_the_browser_and_exits() {
    let (status, out, profile, pid) = run_signal_child("int", "-INT");
    assert_eq!(status.code(), Some(130), "{out}");
    assert!(gone(pid), "browser {pid} still running");
    assert!(
        !std::path::Path::new(&profile).exists(),
        "{profile} left behind"
    );
}

/// `handleSIGTERM`: closes the browser; the program keeps running.
#[tokio::test]
async fn handle_sigterm_closes_the_browser() {
    let (status, out, profile, pid) = run_signal_child("term", "-TERM");
    assert!(out.contains("DISCONNECTED"), "{out}");
    assert_eq!(status.code(), Some(0), "{out}");
    assert!(gone(pid), "browser {pid} still running");
    assert!(
        !std::path::Path::new(&profile).exists(),
        "{profile} left behind"
    );
}

/// `launchServer({ host, port, wsPath })`: the server listens there and
/// accepts only that path.
#[tokio::test]
async fn launch_server_ws_path() {
    let server = Chromium::launch_server_with(chromiumoxide::LaunchServerOptions {
        ws_path: Some("/secret-path".into()),
        host: Some("127.0.0.1".into()),
        port: Some(0),
        ..Default::default()
    })
    .await
    .unwrap();
    let ws = server.ws_endpoint();
    assert!(
        ws.starts_with("ws://127.0.0.1:") && ws.ends_with("/secret-path"),
        "{ws}"
    );
    assert!(server.process().is_some());
    let client = Chromium::connect(&ws).await.unwrap();
    assert!(client.version().await.unwrap().contains("Chrome"));
    let page = client.new_page().await.unwrap();
    page.set_content("<p>ok</p>").await.unwrap();
    client.close().await.unwrap();
    let wrong = ws.replace("/secret-path", "/devtools/browser/guess");
    assert!(
        Chromium::connect(&wrong).await.is_err(),
        "other paths are refused"
    );
    server.close().await.unwrap();
}

/// `browser.bind(title, { host, port, workspaceDir, metadata })`: the bound
/// browser is listed for its workspace until unbound.
#[tokio::test]
async fn bind_options_register_the_browser() {
    let workspace = std::env::temp_dir().join(format!("chromiumoxide-ws-{}", std::process::id()));
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let endpoint = browser
        .bind_with(chromiumoxide::BindOptions {
            title: "my browser".into(),
            host: Some("127.0.0.1".into()),
            port: Some(0),
            workspace_dir: Some(workspace.clone()),
            metadata: serde_json::json!({"purpose": "test"}),
        })
        .await
        .unwrap();
    assert!(endpoint.starts_with("ws://127.0.0.1:"), "{endpoint}");
    let listed = Chromium::bound_browsers().await.unwrap();
    let mine = listed
        .iter()
        .find(|b| b.endpoint == endpoint)
        .unwrap_or_else(|| panic!("not listed: {listed:?}"));
    assert_eq!(mine.title, "my browser");
    assert_eq!(mine.workspace_dir.as_deref(), Some(workspace.as_path()));
    assert_eq!(mine.metadata, serde_json::json!({"purpose": "test"}));
    let client = Chromium::connect(&endpoint).await.unwrap();
    assert!(client.version().await.unwrap().contains("Chrome"));
    client.close().await.unwrap();
    browser.unbind().await.unwrap();
    let listed = Chromium::bound_browsers().await.unwrap();
    assert!(listed.iter().all(|b| b.endpoint != endpoint), "{listed:?}");
    browser.close().await.unwrap();
}

/// `selectors.register(name, script, { contentScript })`: engines run in the
/// page's main world (they see page globals) unless `contentScript: true`.
#[tokio::test]
async fn selector_engines_main_world_unless_content_script() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    // Picks the elements named by the page's own `window.pick`.
    let engine = "({ query(root) { return window.pick ? root.querySelector(window.pick) : null; },\
        queryAll(root) { return window.pick ? [...root.querySelectorAll(window.pick)] : []; } })";
    let selectors = browser.selectors();
    selectors.register("pagepick", engine).unwrap();
    selectors
        .register_with(
            "isolatedpick",
            engine,
            chromiumoxide::SelectorRegisterOptions {
                content_script: true,
            },
        )
        .unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<b>x</b><button onclick='window.clicked=1'>y</button><script>window.pick='button'</script>",
    )
    .await
    .unwrap();
    let main = page.locator("pagepick=any");
    assert_eq!(main.count().await.unwrap(), 1);
    assert_eq!(main.text_content().await.unwrap().as_deref(), Some("y"));
    main.click().await.unwrap();
    assert_eq!(page.evaluate::<i64>("window.clicked").await.unwrap(), 1);
    chromiumoxide::expect(&main)
        .to_have_text("y")
        .await
        .unwrap();
    // Chained under a built-in engine, too.
    assert_eq!(
        page.locator("body >> pagepick=any").count().await.unwrap(),
        1
    );
    // A content script runs isolated: no page globals.
    assert_eq!(page.locator("isolatedpick=any").count().await.unwrap(), 0);
    browser.close().await.unwrap();
}
