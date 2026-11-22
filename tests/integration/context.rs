use chromiumoxide::{
    Chromium, ColorScheme, ContextOptions, HttpCredentials, LaunchOptions, Viewport,
};
use serde_json::Value;

use crate::server::TestServer;

#[tokio::test]
async fn context_emulation_reaches_the_page() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(390, 844)),
            device_scale_factor: Some(3.0),
            has_touch: true,
            user_agent: Some("Mozilla/5.0 (Linux; Android 14; Pixel 7) Test".into()),
            locale: Some("de-DE".into()),
            timezone_id: Some("Asia/Tokyo".into()),
            color_scheme: Some(ColorScheme::Dark),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    // userAgentData only exists in secure contexts; localhost is one.
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>hi</p>", 0);
    page.goto(&server.url("/")).await.unwrap();
    let got: Value = page
        .evaluate(
            "() => ({ w: innerWidth, h: innerHeight, dpr: devicePixelRatio, sw: screen.width, ua: navigator.userAgent, \
             lang: navigator.language, tz: Intl.DateTimeFormat().resolvedOptions().timeZone, \
             dark: matchMedia('(prefers-color-scheme: dark)').matches, touch: navigator.maxTouchPoints > 0, \
             platform: navigator.userAgentData && navigator.userAgentData.platform })",
        )
        .await
        .unwrap();
    assert_eq!(got["w"], 390);
    assert_eq!(got["h"], 844);
    assert_eq!(got["dpr"], 3);
    assert_eq!(got["sw"], 390);
    assert_eq!(got["ua"], "Mozilla/5.0 (Linux; Android 14; Pixel 7) Test");
    assert_eq!(got["lang"], "de-DE");
    assert_eq!(got["tz"], "Asia/Tokyo");
    assert_eq!(got["dark"], true);
    assert_eq!(got["touch"], true);
    assert_eq!(got["platform"], "Android");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn default_context_is_1280_by_720_light() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let got: Value = page
        .evaluate("() => [innerWidth, innerHeight, devicePixelRatio, matchMedia('(prefers-color-scheme: light)').matches]")
        .await
        .unwrap();
    assert_eq!(got, serde_json::json!([1280, 720, 1, true]));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn contexts_are_isolated_and_closable() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let a = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(300, 200)),
            ..Default::default()
        })
        .await
        .unwrap();
    let b = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(600, 400)),
            ..Default::default()
        })
        .await
        .unwrap();
    let (pa, pb) = (a.new_page().await.unwrap(), b.new_page().await.unwrap());
    assert_eq!(pa.evaluate::<u32>("innerWidth").await.unwrap(), 300);
    assert_eq!(pb.evaluate::<u32>("innerWidth").await.unwrap(), 600);
    assert_eq!(browser.contexts().len(), 2);
    assert_eq!(a.pages().len(), 1);
    a.close().await.unwrap();
    assert!(pa.is_closed());
    assert!(pa.evaluate::<u32>("1").await.is_err());
    assert_eq!(pb.evaluate::<u32>("innerWidth").await.unwrap(), 600);
    assert_eq!(browser.contexts().len(), 1);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn evaluate_returns_values_and_reports_exceptions() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    assert_eq!(page.evaluate::<i64>("1 + 2").await.unwrap(), 3);
    assert_eq!(page.evaluate::<String>("() => 'hi'").await.unwrap(), "hi");
    assert_eq!(
        page.evaluate::<i64>("async () => { await new Promise(r => setTimeout(r, 10)); return 7 }")
            .await
            .unwrap(),
        7
    );
    assert_eq!(
        page.evaluate_with::<i64, _>("(x) => x.a * 2", serde_json::json!({"a": 21}))
            .await
            .unwrap(),
        42
    );
    assert_eq!(
        page.evaluate::<Option<i64>>("undefined").await.unwrap(),
        None
    );
    let err = page
        .evaluate::<Value>("() => { throw new Error('boom') }")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("boom"), "{err}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn http_credentials_answer_basic_auth() {
    let server = TestServer::start().await;
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions {
            http_credentials: Some(HttpCredentials {
                username: "user".into(),
                password: "pass".into(),
                origin: None,
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = ctx.new_page().await.unwrap();
    let response = page.goto(&server.url("/protected")).await.unwrap().unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(
        page.evaluate::<String>("document.body.textContent")
            .await
            .unwrap(),
        "secret ok"
    );

    let plain = browser.new_page().await.unwrap();
    let response = plain
        .goto(&server.url("/protected"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status, 401);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn evaluate_runs_statement_strings_like_playwright() {
    // Playwright evaluates a non-function string as a script: statements are
    // allowed, the completion value is returned and a promise is awaited.
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    assert_eq!(page.evaluate::<i64>("const a = 1; a + 1").await.unwrap(), 2);
    page.evaluate::<Value>("document.body.style.margin = 0; window.x = 1")
        .await
        .unwrap();
    assert_eq!(page.evaluate::<i64>("window.x").await.unwrap(), 1);
    assert_eq!(
        page.evaluate::<String>("let s = 'a'; s += 'b'; Promise.resolve(s + 'c')")
            .await
            .unwrap(),
        "abc"
    );
    assert_eq!(
        page.evaluate::<i64>("var f = () => 5; f()").await.unwrap(),
        5
    );
    // expressions and functions keep working
    assert_eq!(page.evaluate::<i64>("1 + 2").await.unwrap(), 3);
    assert_eq!(page.evaluate::<i64>("() => 7").await.unwrap(), 7);
    let err = page
        .evaluate::<Value>("const b = 1; throw new Error('stmt boom')")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("stmt boom"), "{err}");
    let v = page
        .wait_for_function("const t = window.x; t === 1")
        .await
        .unwrap();
    assert_eq!(v, true);
    browser.close().await.unwrap();
}
