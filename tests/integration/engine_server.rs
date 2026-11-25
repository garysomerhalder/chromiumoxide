//! `browserType.launchServer()` for Firefox and WebKit: a browser other
//! programs connect to with `Firefox::connect` / `WebKit::connect`.

use chromiumoxide::{Firefox, ServerAddress, WebKit};

const PAGE: &str = "data:text/html,<title>served</title><p id=p>hi</p>";

#[tokio::test]
async fn firefox_launch_server_serves_clients() {
    let Some(options) = crate::firefox::options() else {
        return;
    };
    let server = Firefox::launch_server_with(
        options,
        ServerAddress {
            ws_path: Some("/firefox-secret".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let ws = server.ws_endpoint();
    assert!(ws.ends_with("/firefox-secret"), "{ws}");
    assert!(server.process().is_some_and(|p| p.pid() > 0));

    let client = Firefox::connect(&ws).await.unwrap();
    let page = client.new_page().await.unwrap();
    page.goto(PAGE).await.unwrap();
    assert_eq!(page.title().await.unwrap(), "served");
    client.close().await.unwrap();

    // A second client after the first disconnected.
    let again = Firefox::connect(&ws).await.unwrap();
    let page = again.new_page().await.unwrap();
    page.goto(PAGE).await.unwrap();
    assert_eq!(
        page.evaluate::<String>("p.textContent").await.unwrap(),
        "hi"
    );
    let wrong = ws.replace("/firefox-secret", "/nope");
    assert!(Firefox::connect(&wrong).await.is_err());
    server.close().await.unwrap();
    assert!(server.process().is_none());

    // Without an address: any free port.
    let server = Firefox::launch_server(crate::firefox::options().unwrap())
        .await
        .unwrap();
    let client = Firefox::connect(&server.ws_endpoint()).await.unwrap();
    assert!(!client.version().await.unwrap().is_empty());
    server.kill().await.unwrap();
}

#[tokio::test]
async fn webkit_launch_server_serves_clients() {
    let Some(options) = crate::webkit::options() else {
        return;
    };
    let server = WebKit::launch_server_with(
        options,
        ServerAddress {
            ws_path: Some("/webkit-secret".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let ws = server.ws_endpoint();
    assert!(ws.ends_with("/webkit-secret"), "{ws}");
    assert!(server.process().is_some_and(|p| p.pid() > 0));

    let client = WebKit::connect(&ws).await.unwrap();
    let page = client.new_page().await.unwrap();
    page.goto(PAGE).await.unwrap();
    assert_eq!(page.title().await.unwrap(), "served");
    client.close().await.unwrap();

    let again = WebKit::connect(&ws).await.unwrap();
    let page = again.new_page().await.unwrap();
    page.goto(PAGE).await.unwrap();
    assert_eq!(
        page.evaluate::<String>("p.textContent").await.unwrap(),
        "hi"
    );
    let wrong = ws.replace("/webkit-secret", "/nope");
    assert!(WebKit::connect(&wrong).await.is_err());
    server.close().await.unwrap();
    assert!(server.process().is_none());

    let server = WebKit::launch_server(crate::webkit::options().unwrap())
        .await
        .unwrap();
    let client = WebKit::connect(&server.ws_endpoint()).await.unwrap();
    assert!(!client.version().await.unwrap().is_empty());
    server.kill().await.unwrap();
}
