use chromiumoxide::{Chromium, LaunchOptions, WaitForState};

#[tokio::test]
async fn page_level_shortcuts_and_wait_for_selector() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<input id=i><input type=checkbox id=c><select id=s><option>a</option><option>b</option></select>
         <button id=b onclick='window.n=(window.n||0)+1'>go</button><p id=p title=t>text</p>
         <script>setTimeout(() => { const d = document.createElement('div'); d.id = 'late'; document.body.append(d) }, 200)</script>",
    )
    .await
    .unwrap();
    page.fill("#i", "hello").await.unwrap();
    assert_eq!(page.input_value("#i").await.unwrap(), "hello");
    page.press("#i", "Backspace").await.unwrap();
    assert_eq!(page.input_value("#i").await.unwrap(), "hell");
    page.check("#c").await.unwrap();
    assert!(page.is_checked("#c").await.unwrap());
    page.uncheck("#c").await.unwrap();
    assert_eq!(page.select_option("#s", &["b"]).await.unwrap(), vec!["b"]);
    page.click("#b").await.unwrap();
    page.dblclick("#b").await.unwrap();
    page.hover("#p").await.unwrap();
    assert_eq!(page.evaluate::<i64>("window.n").await.unwrap(), 3);
    assert_eq!(
        page.text_content("#p").await.unwrap().as_deref(),
        Some("text")
    );
    assert_eq!(page.inner_text("#p").await.unwrap(), "text");
    assert_eq!(
        page.get_attribute("#p", "title").await.unwrap().as_deref(),
        Some("t")
    );
    assert!(page.is_visible("#p").await.unwrap());
    assert!(page.is_enabled("#b").await.unwrap());
    page.wait_for_selector("#late", WaitForState::Attached)
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn drag_to_moves_the_pointer_with_the_button_down() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(
        "<div id=a style='position:absolute;left:10px;top:10px;width:50px;height:50px;background:red'></div>
         <div id=b style='position:absolute;left:200px;top:10px;width:50px;height:50px;background:blue'></div>
         <script>
           window.log = [];
           document.getElementById('a').addEventListener('mousedown', () => log.push('down:a'));
           document.getElementById('b').addEventListener('mouseup', () => log.push('up:b'));
           document.addEventListener('mousemove', e => { if (e.buttons === 1) window.dragged = true });
         </script>",
    )
    .await
    .unwrap();
    page.locator("#a")
        .drag_to(&page.locator("#b"))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<Vec<String>>("window.log").await.unwrap(),
        vec!["down:a", "up:b"]
    );
    assert!(
        page.evaluate::<bool>("window.dragged === true")
            .await
            .unwrap()
    );
    browser.close().await.unwrap();
}
