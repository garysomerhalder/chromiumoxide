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
