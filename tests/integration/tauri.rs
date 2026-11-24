//! Tauri apps (`chromiumoxide::Tauri`): the test app in
//! `tests/fixtures/tauri-app` (build it with `cargo build` there, or point
//! `CHROMIUMOXIDE_TAURI_APP` at a build). On Windows its window is WebView2,
//! driven over CDP like Chromium.

use chromiumoxide::{Tauri, TauriLaunchOptions};

/// The built test app, or `None` (tests skip) unless
/// `CHROMIUMOXIDE_REQUIRE_TAURI` is set.
fn tauri_app() -> Option<std::path::PathBuf> {
    let app = std::env::var_os("CHROMIUMOXIDE_TAURI_APP")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let exe = if cfg!(windows) {
                "cox-tauri-app.exe"
            } else {
                "cox-tauri-app"
            };
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tauri-app/target/debug")
                .join(exe)
        });
    if app.exists() {
        return Some(app);
    }
    assert!(
        std::env::var_os("CHROMIUMOXIDE_REQUIRE_TAURI").is_none(),
        "no Tauri test app at {} (CHROMIUMOXIDE_REQUIRE_TAURI is set)",
        app.display()
    );
    None
}

#[tokio::test]
async fn tauri_window_runs_the_page_api() {
    let Some(app) = tauri_app() else {
        return;
    };
    let app = Tauri::launch(TauriLaunchOptions {
        executable_path: app,
        ..Default::default()
    })
    .await
    .unwrap();
    let window = app.first_window().await.unwrap();
    window
        .wait_for_function("document.getElementById('h')")
        .await
        .unwrap();
    assert_eq!(window.title().await.unwrap(), "Cox Tauri");
    window.locator("#b").click().await.unwrap();
    assert_eq!(window.locator("#h").inner_text().await.unwrap(), "clicked");
    let png = window.screenshot().await.unwrap();
    assert_eq!(&png[1..4], b"PNG");
    // `goto` returns once the page has loaded.
    let server = crate::server::TestServer::start().await;
    server.route("/m.html", "text/html", "<title>served</title><p>here</p>", 0);
    window.goto(&server.url("/m.html")).await.unwrap();
    assert_eq!(
        window.evaluate::<String>("document.title").await.unwrap(),
        "served"
    );
    // WKWebView (macOS) does not open `data:` URLs as the top page.
    if !cfg!(target_os = "macos") {
        window
            .goto("data:text/html,<title>moved</title><p>away</p>")
            .await
            .unwrap();
        assert_eq!(
            window.evaluate::<String>("document.title").await.unwrap(),
            "moved"
        );
    }
    app.close().await.unwrap();
}
