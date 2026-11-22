//! HTML5 drag-and-drop (Chromium's drag interception, as Playwright does) and
//! `locator.drop()` (Playwright 1.63).

use chromiumoxide::{Chromium, DropFile, DropPayload, LaunchOptions};

const DND: &str = r#"<style>div{width:100px;height:100px;margin:20px}</style>
<div id=src draggable=true style="background:red">drag me</div>
<div id=dst style="background:blue"></div>
<script>
  window.log = [];
  src.addEventListener('dragstart', e => { e.dataTransfer.setData('text/plain', 'payload'); log.push('dragstart'); });
  dst.addEventListener('dragenter', e => { e.preventDefault(); log.push('dragenter'); });
  dst.addEventListener('dragover', e => { e.preventDefault(); });
  dst.addEventListener('drop', e => { e.preventDefault(); log.push('drop:' + e.dataTransfer.getData('text/plain')); });
  src.addEventListener('dragend', () => log.push('dragend'));
</script>"#;

#[tokio::test]
async fn html5_drag_to() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(DND).await.unwrap();
    page.locator("#src")
        .drag_to(&page.locator("#dst"))
        .await
        .unwrap();
    let log: Vec<String> = page.evaluate("window.log").await.unwrap();
    assert_eq!(log, ["dragstart", "dragenter", "drop:payload", "dragend"]);

    // The same with the raw mouse.
    page.set_content(DND).await.unwrap();
    let mouse = page.mouse();
    mouse.move_to(70.0, 70.0, 1).await.unwrap();
    mouse.down(chromiumoxide::MouseButton::Left).await.unwrap();
    mouse.move_to(70.0, 190.0, 5).await.unwrap();
    mouse.up(chromiumoxide::MouseButton::Left).await.unwrap();
    let log: Vec<String> = page.evaluate("window.log").await.unwrap();
    assert_eq!(log, ["dragstart", "dragenter", "drop:payload", "dragend"]);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn locator_drop_files_and_data() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        r#"<div id=zone style="width:200px;height:100px">zone</div><div id=no style="width:200px;height:100px">no</div>
<script>
  window.got = null;
  zone.addEventListener('dragover', e => e.preventDefault());
  zone.addEventListener('drop', async e => {
    e.preventDefault();
    const f = e.dataTransfer.files[0];
    window.got = { text: e.dataTransfer.getData('text/plain'), uri: e.dataTransfer.getData('text/uri-list'),
                   name: f && f.name, type: f && f.type, body: f && await f.text() };
  });
  no.addEventListener('dragleave', () => window.left = true);
</script>"#,
    )
    .await
    .unwrap();
    page.locator("#zone")
        .drop(
            DropPayload::default()
                .file(DropFile::new("note.txt", "text/plain", b"hello"))
                .data("text/plain", "hello world")
                .data("text/uri-list", "https://example.com"),
        )
        .await
        .unwrap();
    let got: serde_json::Value = page.wait_for_function("() => window.got").await.unwrap();
    assert_eq!(
        got,
        serde_json::json!({"text": "hello world", "uri": "https://example.com",
                           "name": "note.txt", "type": "text/plain", "body": "hello"})
    );

    let err = page
        .locator("#no")
        .drop(DropPayload::default().data("text/plain", "x"))
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Drop target did not accept the drop — its dragover handler did not call preventDefault()"),
        "{err}"
    );
    assert!(page.evaluate::<bool>("window.left === true").await.unwrap());

    let err = page
        .locator("#zone")
        .drop(DropPayload::default())
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("At least one of \"files\" or \"data\" must be provided."),
        "{err}"
    );
    browser.close().await.unwrap();
}
