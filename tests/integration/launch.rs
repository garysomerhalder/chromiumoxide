use chromiumoxide::{Channel, Chromium, LaunchOptions};

#[tokio::test]
async fn launches_the_discovered_browser_over_a_pipe() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let version = browser.version().await.unwrap();
    assert!(version.contains("Chrome"), "{version}");
    let dir = browser.user_data_dir().unwrap();
    assert!(dir.is_dir());
    browser.close().await.unwrap();
    assert!(!browser.is_connected());
    assert!(!dir.exists(), "profile {} not removed", dir.display());
}

#[tokio::test]
async fn parallel_launches_do_not_interfere() {
    let launches = (0..3).map(|_| Chromium::launch(LaunchOptions::default()));
    let browsers: Vec<_> = futures::future::join_all(launches)
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    let mut dirs: Vec<_> = browsers
        .iter()
        .map(|b| b.user_data_dir().unwrap())
        .collect();
    dirs.sort();
    dirs.dedup();
    assert_eq!(dirs.len(), 3);
    for b in &browsers {
        assert!(b.version().await.unwrap().contains("Chrome"));
    }
    for b in browsers {
        b.close().await.unwrap();
    }
}

#[tokio::test]
async fn launches_full_chromium_in_new_headless_mode() {
    let Some(exe) = crate::full_chromium() else {
        return;
    };
    let browser = Chromium::launch(LaunchOptions {
        executable_path: Some(exe),
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(browser.version().await.unwrap().contains("Chrome"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn launches_over_a_websocket() {
    let browser = Chromium::launch(LaunchOptions {
        channel: Channel::WebSocket,
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(browser.version().await.unwrap().contains("Chrome"));
    let ws = browser.ws_endpoint().expect("ws endpoint");
    assert!(ws.starts_with("ws://127.0.0.1:"), "{ws}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn connects_over_cdp_to_a_running_browser() {
    let owner = Chromium::launch(LaunchOptions {
        channel: Channel::WebSocket,
        ..Default::default()
    })
    .await
    .unwrap();
    let ws = owner.ws_endpoint().unwrap();
    let client = Chromium::connect_over_cdp(&ws).await.unwrap();
    assert!(client.version().await.unwrap().contains("Chrome"));
    owner.close().await.unwrap();
}

#[tokio::test]
async fn missing_explicit_executable_is_reported() {
    let err = Chromium::launch(LaunchOptions {
        executable_path: Some("/definitely/not/here".into()),
        ..Default::default()
    })
    .await
    .err()
    .unwrap();
    assert!(err.to_string().contains("/definitely/not/here"), "{err}");
}

#[tokio::test]
async fn browser_server_outlives_its_clients() {
    let server = Chromium::launch_server(LaunchOptions::default())
        .await
        .unwrap();
    let ws = server.ws_endpoint();
    assert!(ws.starts_with("ws://127.0.0.1:"), "{ws}");

    let a = Chromium::connect(&ws).await.unwrap();
    let ctx = a.new_context(Default::default()).await.unwrap();
    let page = ctx.new_page().await.unwrap();
    page.set_content("<p>a</p>").await.unwrap();
    // Closing a connected browser disconnects and drops its contexts only.
    a.close().await.unwrap();

    let b = Chromium::connect(&ws).await.unwrap();
    assert!(b.version().await.unwrap().contains("Chrome"));
    assert!(
        b.contexts().is_empty(),
        "the other client's contexts are gone"
    );
    let page = b.new_page().await.unwrap();
    page.set_content("<p>b</p>").await.unwrap();
    assert_eq!(
        page.locator("p").text_content().await.unwrap().as_deref(),
        Some("b")
    );
    b.close().await.unwrap();

    server.close().await.unwrap();
    assert!(Chromium::connect(&ws).await.is_err(), "the server is gone");
}
