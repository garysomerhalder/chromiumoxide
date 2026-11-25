//! Typed WebKit commands serialize to what the browser expects.

use chromiumoxide_types::{Command, Method};
use chromiumoxide_webkit_protocol::webkit::protocol::{page, playwright, runtime};

#[test]
fn commands_have_webkit_names_and_params() {
    let nav = page::NavigateParams::new("https://example.com/");
    assert_eq!(nav.identifier(), "Page.navigate");
    assert_eq!(
        serde_json::to_value(&nav).unwrap(),
        serde_json::json!({"url": "https://example.com/"})
    );
    let ctx = playwright::CreateContextParams::default();
    assert_eq!(ctx.identifier(), "Playwright.createContext");
    let eval = runtime::EvaluateParams::new("1 + 1");
    assert_eq!(eval.identifier(), "Runtime.evaluate");
    let _: Option<<runtime::EvaluateParams as Command>::Response> = None;
}

#[test]
fn events_deserialize() {
    let ev: page::EventLoadEventFired =
        serde_json::from_value(serde_json::json!({"timestamp": 1.5})).unwrap();
    assert_eq!(ev.timestamp, 1.5);
}
