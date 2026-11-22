//! `page.clock` (results recorded from Playwright 1.63 on the same steps).

use std::time::Duration;

use chromiumoxide::{Chromium, LaunchOptions};
use serde_json::Value;

const T: u64 = 1_700_000_000_000;

#[tokio::test]
async fn clock_controls_time() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser.new_context(Default::default()).await.unwrap();
    let page = ctx.new_page().await.unwrap();
    page.clock().set_fixed_time(T).await.unwrap();
    page.set_content("<p>x</p>").await.unwrap();
    assert_eq!(page.evaluate::<u64>("Date.now()").await.unwrap(), T);
    assert_eq!(
        page.evaluate::<u64>("new Date().getTime()").await.unwrap(),
        T
    );

    let ctx2 = browser.new_context(Default::default()).await.unwrap();
    let p2 = ctx2.new_page().await.unwrap();
    let clock = p2.clock();
    clock.install(Some(T.into())).await.unwrap();
    p2.set_content("<script>window.fired=[]; setTimeout(()=>fired.push('a'),1500); setTimeout(()=>fired.push('b'), 12500)</script>")
        .await
        .unwrap();
    let elapsed = || async {
        // After resume() the fake clock reports fractional milliseconds.
        p2.evaluate::<f64>("Date.now() - 1700000000000")
            .await
            .unwrap()
    };
    let fired = || async { p2.evaluate::<Value>("window.fired").await.unwrap() };
    clock.pause_at(T + 10_000).await.unwrap();
    assert_eq!(elapsed().await, 10_000.0);
    assert_eq!(fired().await, serde_json::json!(["a"]));
    clock.run_for(2000).await.unwrap();
    assert_eq!(elapsed().await, 12_000.0);
    assert_eq!(fired().await, serde_json::json!(["a"]));
    clock.fast_forward("01:00").await.unwrap();
    assert_eq!(elapsed().await, 72_000.0);
    assert_eq!(fired().await, serde_json::json!(["a", "b"]));
    clock.set_system_time(T + 1_000_000).await.unwrap();
    assert_eq!(elapsed().await, 1_000_000.0);
    clock.resume().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let d = elapsed().await;
    assert!(d > 1_000_000.0 && d < 1_002_000.0, "{d}");
    let err = clock.fast_forward("1:2:3:4").await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "Clock only understands numbers, 'mm:ss' and 'hh:mm:ss'"
    );
    // The clock survives navigations and covers new pages.
    p2.reload().await.unwrap();
    assert!(elapsed().await > 1_000_000.0);
    let p3 = ctx2.new_page().await.unwrap();
    assert!(
        p3.evaluate::<f64>("Date.now() - 1700000000000")
            .await
            .unwrap()
            > 1_000_000.0
    );
    browser.close().await.unwrap();
}
