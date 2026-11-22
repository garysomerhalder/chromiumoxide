//! Behaviour new in Playwright 1.64 and 1.65 (recorded with Playwright 1.65.1
//! against the same Chromium build these tests use).

use std::time::Duration;

use chromiumoxide::{
    AriaSnapshotMode, AriaSnapshotOptions, Browser, Chromium, ContentOptions, Error, LaunchOptions,
    Page, RoleOptions, WebMcpAnnotations, WebMcpTool,
};

use crate::server::TestServer;

async fn page_with(html: &str) -> (Browser, Page) {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(html).await.unwrap();
    (browser, page)
}

/// 1.64: elements inside a hidden iframe are hidden for actions,
/// `isVisible()` and `toBeVisible()`; a `Frame` object is not affected.
#[tokio::test]
async fn elements_in_hidden_iframes_are_hidden() {
    let (browser, page) = page_with(
        r#"<iframe style="visibility:hidden" srcdoc="<button>in hidden</button>"></iframe><iframe id=v srcdoc="<button>in visible</button>"></iframe>"#,
    )
    .await;
    page.wait_for_function(
        "() => [...document.querySelectorAll('iframe')].every(f => f.contentDocument?.querySelector('button'))",
    )
    .await
    .unwrap();
    let hidden = page.frame_locator("iframe").first().locator("button");
    assert!(!hidden.is_visible().await.unwrap());
    assert!(hidden.is_hidden().await.unwrap());
    assert!(
        page.frame_locator("#v")
            .locator("button")
            .is_visible()
            .await
            .unwrap()
    );
    let child = page.main_frame().child_frames().remove(0);
    assert!(child.locator("button").is_visible().await.unwrap());
    let err = hidden
        .click()
        .timeout(Duration::from_millis(600))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Timeout { .. }), "{err:?}");
    browser.close().await.unwrap();
}

/// 1.65: click, fill, check, tap and selectOption wait until the element is
/// not inert; Playwright logs "element is inert".
#[tokio::test]
async fn actions_wait_for_inert_elements() {
    let (browser, page) = page_with(r#"<div inert><button>B</button><input value=x></div>"#).await;
    let err = page
        .locator("button")
        .click()
        .timeout(Duration::from_millis(600))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("element is inert"), "{err}");
    page.evaluate::<()>("void setTimeout(() => document.querySelector('div').inert = false, 300)")
        .await
        .unwrap();
    page.locator("input").fill("y").await.unwrap();
    assert_eq!(page.locator("input").input_value().await.unwrap(), "y");
    browser.close().await.unwrap();
}

/// 1.65: text the browser does not render is left out of accessible names
/// and aria snapshots (here: text directly inside a closed `<details>`).
#[tokio::test]
async fn unrendered_text_is_not_in_aria_snapshots() {
    let (browser, page) = page_with(
        "<details><summary>Sum</summary>inside text</details><button><span slot=x>unslotted</span>ok</button>",
    )
    .await;
    assert_eq!(
        page.locator("body").aria_snapshot().await.unwrap(),
        "- group: Sum\n- button \"unslottedok\""
    );
    browser.close().await.unwrap();
}

const AI_PAGE: &str = r##"<h1>Title</h1><nav><a href="#x">Link</a><ul><li>One</li><li>Two</li></ul></nav><button onclick="window.clicked='main'">Go</button><iframe srcdoc="<p>Inside</p><button onclick='parent.clicked=&quot;frame&quot;'>Frame button</button>"></iframe>"##;

async fn ai_page() -> (Browser, Page) {
    let (browser, page) = page_with(AI_PAGE).await;
    page.wait_for_function("() => window.frames[0]?.document.querySelector('button')")
        .await
        .unwrap();
    (browser, page)
}

/// `ariaSnapshot({ mode, depth, boxes })`, iframes included in 'ai' mode
/// (expected strings recorded from Playwright 1.65.1).
#[tokio::test]
async fn aria_snapshot_options_like_playwright() {
    let (browser, page) = ai_page().await;
    let ai = AriaSnapshotOptions {
        mode: AriaSnapshotMode::Ai,
        ..Default::default()
    };
    assert_eq!(
        page.aria_snapshot_with(ai.clone()).await.unwrap(),
        "- generic [active] [ref=e1]:\n  - heading \"Title\" [level=1] [ref=e2]\n  - navigation [ref=e3]:\n    - link \"Link\" [ref=e4] [cursor=pointer]:\n      - /url: \"#x\"\n    - list [ref=e5]:\n      - listitem [ref=e6]: One\n      - listitem [ref=e7]: Two\n  - button \"Go\" [ref=e8]\n  - iframe [ref=e9]:\n    - generic [ref=f1e1]:\n      - paragraph [ref=f1e2]: Inside\n      - button \"Frame button\" [ref=f1e3]"
    );
    assert_eq!(
        page.aria_snapshot_with(AriaSnapshotOptions {
            depth: Some(1),
            ..ai.clone()
        })
        .await
        .unwrap(),
        "- generic [active] [ref=e1]:\n  - heading \"Title\" [level=1] [ref=e2]\n  - navigation [ref=e3]\n  - button \"Go\" [ref=e8]\n  - iframe [ref=e9]:\n    - generic [ref=f1e1]:\n      - paragraph [ref=f1e2]: Inside\n      - button \"Frame button\" [ref=f1e3]"
    );
    assert_eq!(
        page.aria_snapshot_with(AriaSnapshotOptions {
            depth: Some(1),
            ..Default::default()
        })
        .await
        .unwrap(),
        "- heading \"Title\" [level=1]\n- navigation:\n  - link \"Link\":\n    - /url: \"#x\"\n  - list\n- button \"Go\"\n- iframe"
    );
    assert_eq!(
        page.locator("nav").aria_snapshot_with(ai).await.unwrap(),
        "- navigation [ref=e3]:\n  - link \"Link\" [ref=e4] [cursor=pointer]:\n    - /url: \"#x\"\n  - list [ref=e5]:\n    - listitem [ref=e6]: One\n    - listitem [ref=e7]: Two"
    );
    assert_eq!(
        page.locator("h1")
            .aria_snapshot_with(AriaSnapshotOptions {
                boxes: true,
                ..Default::default()
            })
            .await
            .unwrap(),
        "- heading \"Title\" [level=1] [box=8,8,1264,37]"
    );
    browser.close().await.unwrap();
}

/// 1.64: `page.getByRef(ref)` finds an element by its 'ai' snapshot ref,
/// including refs inside iframes (`f1e3`).
#[tokio::test]
async fn get_by_ref_targets_ai_snapshot_refs() {
    let (browser, page) = ai_page().await;
    page.aria_snapshot_with(AriaSnapshotOptions {
        mode: AriaSnapshotMode::Ai,
        ..Default::default()
    })
    .await
    .unwrap();
    let go = page.get_by_ref("e8");
    assert_eq!(go.selector(), "aria-ref=e8");
    assert_eq!(
        go.to_playwright_string().await.unwrap(),
        "locator('aria-ref=e8')"
    );
    go.click().await.unwrap();
    assert_eq!(
        page.evaluate::<String>("window.clicked").await.unwrap(),
        "main"
    );
    page.get_by_ref("f1e3").click().await.unwrap();
    assert_eq!(
        page.evaluate::<String>("window.clicked").await.unwrap(),
        "frame"
    );
    let err = page
        .get_by_ref("e999")
        .click()
        .timeout(Duration::from_millis(500))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Timeout { .. }), "{err:?}");
    browser.close().await.unwrap();
}

/// 1.64: `locator.within(parent)`; relative locators resolve per parent.
#[tokio::test]
async fn locator_within_scopes_to_each_parent() {
    let (browser, page) = page_with(
        "<table><tr><td>a1</td><td>a2</td><td>a3</td></tr><tr><td>b1</td><td>b2</td><td>b3</td></tr></table><div role=dialog><button>Save</button></div><button>Save</button>",
    )
    .await;
    let third = page
        .get_by_role("cell", RoleOptions::default())
        .nth(2)
        .within(&page.get_by_role("row", RoleOptions::default()));
    assert_eq!(third.all_text_contents().await.unwrap(), ["a3", "b3"]);
    let save = page
        .get_by_role(
            "button",
            RoleOptions {
                name: Some("Save".into()),
                ..Default::default()
            },
        )
        .within(&page.get_by_role("dialog", RoleOptions::default()));
    assert_eq!(save.count().await.unwrap(), 1);
    assert_eq!(
        save.to_playwright_string().await.unwrap(),
        "getByRole('dialog').locator(getByRole('button', { name: 'Save' }))"
    );
    browser.close().await.unwrap();
}

/// 1.64: `content({ includeShadow })` serializes open shadow roots as
/// declarative shadow DOM.
#[tokio::test]
async fn content_can_include_shadow_roots() {
    let (browser, page) = page_with(
        "<!DOCTYPE html><div id=host></div><script>const r = host.attachShadow({mode:'open'}); r.innerHTML = '<b>shadow</b><span id=inner></span>'; r.getElementById('inner').attachShadow({mode:'open'}).innerHTML='<i>deep</i>';</script>",
    )
    .await;
    let script = "<script>const r = host.attachShadow({mode:'open'}); r.innerHTML = '<b>shadow</b><span id=inner></span>'; r.getElementById('inner').attachShadow({mode:'open'}).innerHTML='<i>deep</i>';</script>";
    assert_eq!(
        page.content().await.unwrap(),
        format!(
            "<!DOCTYPE html><html><head></head><body><div id=\"host\"></div>{script}</body></html>"
        )
    );
    let with_shadow = format!(
        "<!DOCTYPE html><html><head></head><body><div id=\"host\"><template shadowrootmode=\"open\"><b>shadow</b><span id=\"inner\"><template shadowrootmode=\"open\"><i>deep</i></template></span></template></div>{script}</body></html>"
    );
    let options = ContentOptions {
        include_shadow: true,
    };
    assert_eq!(
        page.content_with(options.clone()).await.unwrap(),
        with_shadow
    );
    assert_eq!(
        page.main_frame().content_with(options).await.unwrap(),
        with_shadow
    );
    browser.close().await.unwrap();
}

const WEBMCP_PAGE: &str = "<p>tools</p><script>document.modelContext.registerTool({ name: 'add', description: 'Adds', inputSchema: { type: 'object', properties: { a: { type: 'number' }, b: { type: 'number' } } }, annotations: { readOnlyHint: true }, execute: ({ a, b }) => ({ content: [{ type: 'text', text: String(a + b) }] }) });</script>";

/// 1.64: `page.webmcp` lists and calls the page's WebMCP tools (Chromium
/// needs `--enable-features=WebMCP` and a secure origin).
#[tokio::test]
async fn webmcp_lists_and_calls_page_tools() {
    let server = TestServer::start().await;
    server.route("/tools", "text/html", WEBMCP_PAGE, 0);
    let browser = Chromium::launch(LaunchOptions {
        args: vec!["--enable-features=WebMCP".into()],
        ..Default::default()
    })
    .await
    .unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/tools")).await.unwrap();
    let tools = page.webmcp().tools().await.unwrap();
    assert_eq!(
        tools,
        vec![WebMcpTool {
            name: "add".into(),
            description: "Adds".into(),
            input_schema: Some(
                serde_json::json!({"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}})
            ),
            annotations: Some(WebMcpAnnotations {
                read_only: true,
                ..Default::default()
            }),
        }]
    );
    assert_eq!(
        page.webmcp()
            .call_tool("add", serde_json::json!({"a": 2, "b": 40}))
            .await
            .unwrap(),
        serde_json::json!({"content": [{"type": "text", "text": "42"}]})
    );
    let err = page
        .webmcp()
        .call_tool("nope", serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("No WebMCP tool named \"nope\". Available tools: add."),
        "{err}"
    );
    assert_eq!(page.main_frame().webmcp().tools().await.unwrap().len(), 1);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn webmcp_needs_the_chromium_feature() {
    let (browser, page) = page_with("<p>x</p>").await;
    let err = page.webmcp().tools().await.unwrap_err();
    assert!(
        err.to_string().contains(
            "WebMCP is not enabled. Launch the browser with the \"--enable-features=WebMCP\" argument."
        ),
        "{err}"
    );
    browser.close().await.unwrap();
}
