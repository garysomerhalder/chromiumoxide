use chromiumoxide::{Chromium, LaunchOptions};
use serde_json::{Value, json};

#[tokio::test]
async fn evaluate_handle_properties_and_disposal() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let obj = page
        .evaluate_handle("() => ({ a: 1, nested: { b: 'x' }, list: [1, 2] })")
        .await
        .unwrap();
    assert_eq!(
        obj.json_value::<Value>().await.unwrap(),
        json!({"a": 1, "nested": {"b": "x"}, "list": [1, 2]})
    );
    let nested = obj.get_property("nested").await.unwrap();
    assert_eq!(
        nested.json_value::<Value>().await.unwrap(),
        json!({"b": "x"})
    );
    let props = obj.get_properties().await.unwrap();
    assert_eq!(
        props.keys().cloned().collect::<Vec<_>>(),
        vec!["a", "list", "nested"]
    );
    let sum: i64 = obj
        .evaluate_with("(o, k) => o.a + o.list.length * k", 10)
        .await
        .unwrap();
    assert_eq!(sum, 21);
    let window = page.evaluate_handle("window").await.unwrap();
    let doc = window.evaluate_handle("w => w.document").await.unwrap();
    assert_eq!(
        doc.evaluate::<String>("d => d.nodeName").await.unwrap(),
        "#document"
    );
    // Like Playwright: any DOM node (including the document) is an element handle; window is not.
    assert!(doc.as_element().is_some());
    assert!(window.as_element().is_none());
    let body = page.evaluate_handle("document.body").await.unwrap();
    assert!(body.as_element().is_some());
    obj.dispose().await.unwrap();
    assert!(obj.json_value::<Value>().await.is_err());
    browser.close().await.unwrap();
}
