use chromiumoxide::{Chromium, ContextOptions, LaunchOptions, ScriptTag};
use serde_json::{Value, json};

use crate::server::TestServer;

const READS_FOO: &str = "<script>document.title = String(window.foo)</script>";

#[tokio::test]
async fn init_scripts_run_before_page_scripts_on_every_navigation() {
    let server = TestServer::start().await;
    server.route("/", "text/html", READS_FOO, 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.add_init_script("window.foo = 'page'").await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    assert_eq!(page.title().await.unwrap(), "page");
    page.reload().await.unwrap();
    assert_eq!(page.title().await.unwrap(), "page");
    page.add_init_script_with("(x) => { window.bar = x.n * 2 }", json!({"n": 21}))
        .await
        .unwrap();
    page.reload().await.unwrap();
    assert_eq!(page.evaluate::<i64>("window.bar").await.unwrap(), 42);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn context_init_scripts_reach_new_and_existing_pages() {
    let server = TestServer::start().await;
    server.route("/", "text/html", READS_FOO, 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let existing = ctx.new_page().await.unwrap();
    ctx.add_init_script("window.foo = 'ctx'").await.unwrap();
    let fresh = ctx.new_page().await.unwrap();
    for page in [&existing, &fresh] {
        page.goto(&server.url("/")).await.unwrap();
        assert_eq!(page.title().await.unwrap(), "ctx");
    }
    browser.close().await.unwrap();
}

#[tokio::test]
async fn add_script_tag_content_url_and_module() {
    let server = TestServer::start().await;
    server.route(
        "/lib.js",
        "application/javascript",
        "window.fromUrl = 'yes'",
        200,
    );
    server.route("/", "text/html", "<p>x</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    page.add_script_tag(ScriptTag::content("window.fromContent = 1"))
        .await
        .unwrap();
    assert_eq!(page.evaluate::<i64>("window.fromContent").await.unwrap(), 1);
    page.add_script_tag(ScriptTag::url(&server.url("/lib.js")))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<String>("window.fromUrl").await.unwrap(),
        "yes"
    );
    page.add_script_tag(ScriptTag::content("window.isModule = typeof import.meta").module())
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<String>("window.isModule").await.unwrap(),
        "object"
    );
    let err = page
        .add_script_tag(ScriptTag::url(&server.url("/missing.js")))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Failed to load script"), "{err}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn exposed_functions_are_callable_and_survive_navigation() {
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>x</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.expose_function("compute", |args: Vec<Value>| async move {
        let a = args[0].as_i64().unwrap();
        let b = args[1].as_i64().unwrap();
        if b == 0 {
            return Err("division by zero".to_owned());
        }
        Ok(json!(a * b))
    })
    .await
    .unwrap();
    assert_eq!(
        page.evaluate::<i64>("async () => await window.compute(6, 7)")
            .await
            .unwrap(),
        42
    );
    page.goto(&server.url("/")).await.unwrap();
    assert_eq!(
        page.evaluate::<i64>("() => window.compute(2, 3)")
            .await
            .unwrap(),
        6
    );
    let err = page
        .evaluate::<Value>("() => window.compute(1, 0)")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("division by zero"), "{err}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn context_exposed_functions_reach_every_page() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let existing = ctx.new_page().await.unwrap();
    ctx.expose_function("hello", |args: Vec<Value>| async move {
        Ok(json!(format!("hi {}", args[0].as_str().unwrap())))
    })
    .await
    .unwrap();
    let fresh = ctx.new_page().await.unwrap();
    for page in [&existing, &fresh] {
        assert_eq!(
            page.evaluate::<String>("() => window.hello('bob')")
                .await
                .unwrap(),
            "hi bob"
        );
    }
    browser.close().await.unwrap();
}

#[tokio::test]
async fn exposed_bindings_receive_their_source_frame() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.expose_binding(
        "whoami",
        |source: chromiumoxide::BindingSource, args: Vec<Value>| async move {
            let url = source.frame.url();
            Ok(json!(format!(
                "{}|{}|{}",
                source.page.target_id().len() > 0,
                url,
                args[0]
            )))
        },
    )
    .await
    .unwrap();
    page.set_content("<iframe name=f srcdoc='<p>x</p>'></iframe>")
        .await
        .unwrap();
    page.wait_for_function(
        "() => window.frames[0] && window.frames[0].document.readyState === 'complete'",
    )
    .await
    .unwrap();
    let main: String = page.evaluate("() => window.whoami(1)").await.unwrap();
    assert_eq!(main, "true|about:blank|1");
    let child: String = page
        .frame("f")
        .unwrap()
        .evaluate("() => window.whoami(2)")
        .await
        .unwrap();
    assert_eq!(child, "true|about:srcdoc|2");
    browser.close().await.unwrap();
}
