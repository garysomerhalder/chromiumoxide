use chromiumoxide::{Chromium, ContextOptions, LaunchOptions, MouseButton};
use serde_json::Value;

const RECORDER: &str = "<input id=i style='position:absolute;left:10px;top:10px;width:200px;height:30px'>
<button id=b style='position:absolute;left:10px;top:60px;width:100px;height:40px' onclick='window.clicks=(window.clicks||0)+1' ondblclick='window.dbl=true'>b</button>
<div style='height:3000px'></div>
<script>
window.keys = [];
document.getElementById('i').addEventListener('keydown', e => keys.push([e.key, e.code, e.shiftKey, e.ctrlKey, e.metaKey]));
window.mouse = [];
for (const t of ['mousedown', 'mouseup', 'click', 'contextmenu'])
  document.addEventListener(t, e => mouse.push([t, e.button, e.clientX, e.clientY]));
window.touches = 0;
document.addEventListener('touchstart', () => touches++);
</script>";

#[tokio::test]
async fn keyboard_types_presses_and_inserts() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(RECORDER).await.unwrap();
    page.evaluate::<()>("() => document.getElementById('i').focus()")
        .await
        .unwrap();
    let kb = page.keyboard();
    kb.type_text("Hello ü!").await.unwrap();
    assert_eq!(
        page.evaluate::<String>("document.getElementById('i').value")
            .await
            .unwrap(),
        "Hello ü!"
    );
    kb.press("Shift+KeyA").await.unwrap();
    kb.press("ControlOrMeta+a").await.unwrap();
    kb.press("Backspace").await.unwrap();
    assert_eq!(
        page.evaluate::<String>("document.getElementById('i').value")
            .await
            .unwrap(),
        ""
    );
    kb.insert_text("嗨").await.unwrap();
    assert_eq!(
        page.evaluate::<String>("document.getElementById('i').value")
            .await
            .unwrap(),
        "嗨"
    );
    let keys: Vec<Value> = page.evaluate("window.keys").await.unwrap();
    assert!(
        keys.contains(&serde_json::json!(["A", "KeyA", true, false, false])),
        "{keys:?}"
    );
    assert!(
        // ControlOrMeta: Meta on macOS, Control elsewhere.
        keys.contains(&serde_json::json!([
            "a",
            "KeyA",
            false,
            !cfg!(target_os = "macos"),
            cfg!(target_os = "macos")
        ])),
        "{keys:?}"
    );
    assert!(
        kb.press("NoSuchKey")
            .await
            .unwrap_err()
            .to_string()
            .contains("Unknown key")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn mouse_clicks_double_clicks_right_clicks_and_wheels() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(RECORDER).await.unwrap();
    let mouse = page.mouse();
    mouse.click(60.0, 80.0).await.unwrap();
    assert_eq!(page.evaluate::<i64>("window.clicks").await.unwrap(), 1);
    mouse.dblclick(60.0, 80.0).await.unwrap();
    assert!(page.evaluate::<bool>("window.dbl === true").await.unwrap());
    mouse.move_to(20.0, 20.0, 3).await.unwrap();
    mouse.down(MouseButton::Right).await.unwrap();
    mouse.up(MouseButton::Right).await.unwrap();
    let events: Vec<Value> = page.evaluate("window.mouse").await.unwrap();
    assert!(
        events.contains(&serde_json::json!(["contextmenu", 2, 20, 20])),
        "{events:?}"
    );
    mouse.wheel(0.0, 500.0).await.unwrap();
    page.wait_for_function("() => window.scrollY >= 500")
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn touchscreen_taps() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions {
            has_touch: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = ctx.new_page().await.unwrap();
    page.set_content(RECORDER).await.unwrap();
    page.touchscreen().tap(60.0, 80.0).await.unwrap();
    assert_eq!(page.evaluate::<i64>("window.touches").await.unwrap(), 1);
    browser.close().await.unwrap();
}
