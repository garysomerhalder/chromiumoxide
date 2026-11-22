//! Behaviour new in Playwright 1.64 and 1.65 (recorded with Playwright 1.65.1
//! against the same Chromium build these tests use).

use std::time::Duration;

use chromiumoxide::{Browser, Chromium, Error, LaunchOptions, Page};

async fn page_with(html: &str) -> (Browser, Page) {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(html).await.unwrap();
    (browser, page)
}

/// 1.64: elements inside a hidden iframe are hidden for actions,
/// `isVisible()` and `toBeVisible()`; a `Frame` object is not affected.
#[tokio::test]
async fn elements_in_hidden_iframes_are_hidden() {
    let (browser, page) = page_with(
        r#"<iframe style="visibility:hidden" srcdoc="<button>in hidden</button>"></iframe><iframe id=v srcdoc="<button>in visible</button>"></iframe>"#,
    )
    .await;
    page.wait_for_function(
        "() => [...document.querySelectorAll('iframe')].every(f => f.contentDocument?.querySelector('button'))",
    )
    .await
    .unwrap();
    let hidden = page.frame_locator("iframe").first().locator("button");
    assert!(!hidden.is_visible().await.unwrap());
    assert!(hidden.is_hidden().await.unwrap());
    assert!(
        page.frame_locator("#v")
            .locator("button")
            .is_visible()
            .await
            .unwrap()
    );
    let child = page.main_frame().child_frames().remove(0);
    assert!(child.locator("button").is_visible().await.unwrap());
    let err = hidden
        .click()
        .timeout(Duration::from_millis(600))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Timeout { .. }), "{err:?}");
    browser.close().await.unwrap();
}

/// 1.65: click, fill, check, tap and selectOption wait until the element is
/// not inert; Playwright logs "element is inert".
#[tokio::test]
async fn actions_wait_for_inert_elements() {
    let (browser, page) =
        page_with(r#"<div inert><button>B</button><input value=x></div>"#).await;
    let err = page
        .locator("button")
        .click()
        .timeout(Duration::from_millis(600))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("element is inert"), "{err}");
    page.evaluate::<()>(
        "setTimeout(() => document.querySelector('div').inert = false, 300)",
    )
    .await
    .unwrap();
    page.locator("input").fill("y").await.unwrap();
    assert_eq!(page.locator("input").input_value().await.unwrap(), "y");
    browser.close().await.unwrap();
}

/// 1.65: text the browser does not render is left out of accessible names
/// and aria snapshots (here: text directly inside a closed `<details>`).
#[tokio::test]
async fn unrendered_text_is_not_in_aria_snapshots() {
    let (browser, page) = page_with(
        "<details><summary>Sum</summary>inside text</details><button><span slot=x>unslotted</span>ok</button>",
    )
    .await;
    assert_eq!(
        page.locator("body").aria_snapshot().await.unwrap(),
        "- group: Sum\n- button \"unslottedok\""
    );
    browser.close().await.unwrap();
}
