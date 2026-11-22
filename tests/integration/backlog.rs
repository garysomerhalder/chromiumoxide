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
