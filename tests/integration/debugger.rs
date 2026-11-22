//! `context.debugger`, `page.pause()` and `page.requestGC()` (Playwright 1.63 API).

use std::time::Duration;

use chromiumoxide::{Chromium, LaunchOptions};
use futures::StreamExt;

async fn wait_paused(d: &chromiumoxide::Debugger) -> chromiumoxide::PausedDetails {
    for _ in 0..200 {
        if let Some(p) = d.paused_details() {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never paused");
}

#[tokio::test]
async fn debugger_pauses_before_actions() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser.new_context(Default::default()).await.unwrap();
    let page = ctx.new_page().await.unwrap();
    page.set_content("<button onclick='window.n=(window.n||0)+1'>Go</button><input>")
        .await
        .unwrap();
    let dbg = ctx.debugger();
    let mut changes = dbg.paused_state_changes();
    assert!(dbg.paused_details().is_none());
    assert_eq!(
        dbg.resume().await.unwrap_err().to_string(),
        "Debugger is not paused"
    );
    assert_eq!(
        dbg.next().await.unwrap_err().to_string(),
        "Debugger is not paused"
    );

    dbg.request_pause().await.unwrap();
    let actions = tokio::spawn({
        let page = page.clone();
        async move {
            page.locator("button").click().await.unwrap();
            page.locator("input").fill("x").await.unwrap();
        }
    });
    let p = wait_paused(&dbg).await;
    assert_eq!(p.title, "Click locator('button')");
    assert_eq!(changes.next().await, Some(true));
    assert_eq!(
        dbg.request_pause().await.unwrap_err().to_string(),
        "Debugger is already paused"
    );
    assert!(
        page.evaluate::<Option<i64>>("window.n")
            .await
            .unwrap()
            .is_none(),
        "not clicked yet"
    );
    dbg.next().await.unwrap();
    assert_eq!(changes.next().await, Some(false));
    let p = wait_paused(&dbg).await;
    assert_eq!(p.title, "Fill \"x\" locator('input')");
    assert_eq!(page.evaluate::<i64>("window.n").await.unwrap(), 1);
    dbg.resume().await.unwrap();
    actions.await.unwrap();
    assert_eq!(page.locator("input").input_value().await.unwrap(), "x");

    // page.pause() stops right there until resumed.
    let pause = tokio::spawn({
        let page = page.clone();
        async move { page.pause().await }
    });
    let p = wait_paused(&dbg).await;
    assert_eq!(p.title, "Pause");
    dbg.resume().await.unwrap();
    pause.await.unwrap().unwrap();
    assert!(dbg.paused_details().is_none());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn request_gc_collects_unreachable_objects() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.evaluate::<()>("() => { let suspect = { big: new Array(1000).fill(1) }; globalThis.suspectWeakRef = new WeakRef(suspect); suspect = null; }")
        .await
        .unwrap();
    page.request_gc().await.unwrap();
    assert!(
        page.evaluate::<bool>("() => !globalThis.suspectWeakRef.deref()")
            .await
            .unwrap()
    );
    browser.close().await.unwrap();
}
