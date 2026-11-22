//! Popups, context page events and downloads (expectations recorded from
//! Playwright 1.63).

use chromiumoxide::{Chromium, ContextEvent, ContextOptions, LaunchOptions};
use futures::StreamExt;

use crate::server::TestServer;

#[tokio::test]
async fn popups_and_context_pages() {
    let server = TestServer::start().await;
    server.route("/start", "text/html", "<title>start</title>", 0);
    server.route("/popped", "text/html", "<title>pop</title><p>popped</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let mut events = context.events();
    let page = context.new_page().await.unwrap();
    match events.next().await.unwrap() {
        ContextEvent::Page(p) => assert_eq!(p, page),
        other => panic!("{other:?}"),
    }
    page.goto(&server.url("/start")).await.unwrap();
    assert!(page.opener().is_none());

    let url = server.url("/popped");
    let popup = page
        .wait_for_popup()
        .run(page.evaluate::<()>(&format!("() => {{ window.open('{url}') }}")))
        .await
        .unwrap();
    popup
        .wait_for_load_state(chromiumoxide::WaitUntil::Load, None)
        .await
        .unwrap();
    assert_eq!(popup.url(), url);
    assert_eq!(popup.title().await.unwrap(), "pop");
    assert_eq!(popup.opener().unwrap(), page);
    assert_eq!(context.pages().len(), 2);
    match events.next().await.unwrap() {
        ContextEvent::Page(p) => assert_eq!(p, popup),
        other => panic!("{other:?}"),
    }

    // context.wait_for_page
    let third = context
        .wait_for_page()
        .run(page.evaluate::<()>("() => { window.open('about:blank') }"))
        .await
        .unwrap();
    assert_eq!(third.opener().unwrap(), page);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn downloads_like_playwright() {
    let server = TestServer::start().await;
    server.respond(
        "/download",
        "200 OK",
        &[
            ("Content-Type", "application/octet-stream"),
            ("Content-Disposition", "attachment; filename=file.txt"),
        ],
        "Hello world",
    );
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(&format!(
        "<a id=d href='{}'>dl</a><a id=s href='{}'>slow</a>",
        server.url("/download"),
        server.url("/slow-download")
    ))
    .await
    .unwrap();
    let download = page
        .wait_for_download()
        .run(page.click("#d"))
        .await
        .unwrap();
    assert_eq!(download.url(), server.url("/download"));
    assert_eq!(download.suggested_filename(), "file.txt");
    assert_eq!(download.page(), page);
    let path = download.path().await.unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Hello world");
    assert_eq!(download.failure().await.unwrap(), None);
    let dir = std::env::temp_dir().join(format!("chromiumoxide-dl-{}", std::process::id()));
    let target = dir.join("nested").join("saved.txt");
    download.save_as(&target).await.unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "Hello world");
    let _ = std::fs::remove_dir_all(&dir);
    download.delete().await.unwrap();
    assert!(!path.exists());

    let slow = page
        .wait_for_download()
        .run(page.click("#s"))
        .await
        .unwrap();
    assert_eq!(slow.suggested_filename(), "slow.bin");
    slow.cancel().await.unwrap();
    assert_eq!(slow.failure().await.unwrap().as_deref(), Some("canceled"));

    // Without acceptDownloads the download is reported but not saved.
    let context = browser
        .new_context(ContextOptions {
            accept_downloads: false,
            ..Default::default()
        })
        .await
        .unwrap();
    let p2 = context.new_page().await.unwrap();
    p2.set_content(&format!("<a href='{}'>dl</a>", server.url("/download")))
        .await
        .unwrap();
    let denied = p2.wait_for_download().run(p2.click("a")).await.unwrap();
    let msg = "Pass { acceptDownloads: true } when you are creating your browser context.";
    assert_eq!(denied.failure().await.unwrap().as_deref(), Some(msg));
    assert!(denied.path().await.unwrap_err().to_string().contains(msg));
    browser.close().await.unwrap();
}
