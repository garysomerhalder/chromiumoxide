use std::time::Duration;

use chromiumoxide::{Chromium, LaunchOptions, PageEvent};
use futures::StreamExt;

#[tokio::test]
async fn console_messages_stream_and_history() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let mut events = page.events();
    page.evaluate::<()>(
        "() => { console.log('hello', 42, true, null, undefined, {a: 1, b: [1,2]}, [1, 'x'], document.body); console.warn('w'); }",
    )
    .await
    .unwrap();
    let mut got = Vec::new();
    while got.len() < 2 {
        let e = tokio::time::timeout(Duration::from_secs(5), events.next())
            .await
            .unwrap()
            .unwrap();
        if let PageEvent::Console(m) = e {
            got.push((m.kind().to_owned(), m.text().to_owned()));
        }
    }
    assert_eq!(
        got[0],
        (
            "log".to_owned(),
            "hello 42 true null undefined {a: 1, b: Array(2)} [1, x] JSHandle@node".to_owned()
        )
    );
    assert_eq!(got[1], ("warning".to_owned(), "w".to_owned()));
    let history: Vec<String> = page
        .console_messages()
        .iter()
        .map(|m| m.text().to_owned())
        .collect();
    assert!(history.contains(&"w".to_owned()), "{history:?}");
    let msg = page
        .wait_for_console_message(|m| m.text() == "later")
        .run(page.evaluate::<()>("() => { setTimeout(() => console.log('later'), 50) }"))
        .await
        .unwrap();
    assert_eq!(msg.kind(), "log");
    assert_eq!(msg.args().len(), 1);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn page_errors_are_reported() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let error = page
        .wait_for_page_error()
        .run(page.set_content("<script>setTimeout(() => { throw new Error('boom') }, 0)</script>"))
        .await
        .unwrap();
    assert_eq!(
        (error.name.as_str(), error.message.as_str()),
        ("Error", "boom")
    );
    assert_eq!(page.page_errors().len(), 1);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn dialogs_are_dismissed_unless_handled() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    // No handler: Playwright dismisses dialogs automatically.
    assert!(
        !page
            .evaluate::<bool>("() => confirm('sure?')")
            .await
            .unwrap()
    );
    assert_eq!(
        page.evaluate::<Option<String>>("() => prompt('name?', 'def')")
            .await
            .unwrap(),
        None
    );
    page.on_dialog(|dialog| async move {
        assert_eq!(dialog.kind(), "prompt");
        assert_eq!(dialog.message(), "name?");
        assert_eq!(dialog.default_value(), "def");
        dialog.accept(Some("typed")).await.unwrap();
    });
    assert_eq!(
        page.evaluate::<String>("() => prompt('name?', 'def')")
            .await
            .unwrap(),
        "typed"
    );
    browser.close().await.unwrap();
}
