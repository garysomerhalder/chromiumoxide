//! Every generated command and event is usable through the typed escape hatch.

use chromiumoxide::cdp::browser_protocol::browser::GetVersionParams;
use chromiumoxide::cdp::browser_protocol::page::{EventLoadEventFired, NavigateParams};
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use chromiumoxide::{Chromium, LaunchOptions};
use futures::StreamExt;

#[tokio::test]
async fn typed_commands_and_events() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let version = browser.execute(GetVersionParams::default()).await.unwrap();
    assert!(version.product.contains("Chrome"));

    let page = browser.new_page().await.unwrap();
    let mut loads = page.events::<EventLoadEventFired>();
    page.execute(NavigateParams::new("data:text/html,<p>x</p>"))
        .await
        .unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(10), loads.next())
        .await
        .unwrap()
        .unwrap();
    assert!(*event.timestamp.inner() > 0.0);

    let r = page.execute(EvaluateParams::new("6 * 7")).await.unwrap();
    assert_eq!(r.result.value, Some(serde_json::json!(42)));
    browser.close().await.unwrap();
}
