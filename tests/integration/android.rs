//! Playwright's `_android` against the fake adb server (`fake_adb.rs`):
//! devices, shell, sockets, push, installApk, screenshot, WebViews and
//! `launchBrowser`.

use std::time::Duration;

use chromiumoxide::{Android, AndroidDevicesOptions, AndroidLaunchOptions};
use futures::StreamExt;

use crate::fake_adb::{FakeAdb, SERIAL};

#[tokio::test]
async fn android_device_shell_files_webviews_and_browser() {
    let adb = FakeAdb::start().await;
    let devices = Android::devices(AndroidDevicesOptions {
        host: Some("127.0.0.1".into()),
        port: Some(adb.port),
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(devices.len(), 1);
    let device = &devices[0];
    assert_eq!(device.serial(), SERIAL);
    assert_eq!(device.model(), "Fake Pixel");

    // shell, push, installApk, screenshot.
    assert_eq!(device.shell("echo hello").await.unwrap(), b"hello\n");
    device
        .push(b"pushed bytes", "/data/local/tmp/file.txt", 0o644)
        .await
        .unwrap();
    assert_eq!(
        adb.file("/data/local/tmp/file.txt").as_deref(),
        Some(&b"pushed bytes"[..])
    );
    device.install_apk(&[7u8; 3000], None).await.unwrap();
    assert_eq!(adb.installs(), [3000]);
    let png = device.screenshot().await.unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

    // open(): a raw socket to a device service.
    let socket = device.open("shell:cat").await.unwrap();
    let mut data = socket.data();
    socket.write(b"ping").await.unwrap();
    let echoed = tokio::time::timeout(Duration::from_secs(5), data.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed, b"ping");
    socket.close().await.unwrap();

    // WebViews: listed with their pid and package, as pages.
    let views = device.web_views().await.unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].pid(), 4242);
    assert_eq!(views[0].pkg(), "com.example.app");
    let view = device
        .web_view("com.example.app", Duration::from_secs(10))
        .await
        .unwrap();
    let page = view.page().await.unwrap();
    page.set_content("<h1>webview</h1>").await.unwrap();
    assert_eq!(
        page.locator("h1").text_content().await.unwrap().as_deref(),
        Some("webview")
    );

    // launchBrowser(): Chrome with Playwright's command line, as a context.
    let mut events = device.events();
    let context = device
        .launch_browser(AndroidLaunchOptions::default())
        .await
        .unwrap();
    let log = adb.shell_log();
    assert!(
        log.iter().any(|c| c == "am force-stop com.android.chrome"),
        "{log:?}"
    );
    assert!(
        log.iter().any(|c| c.starts_with(
            "am start -a android.intent.action.VIEW -d about:blank com.android.chrome"
        )),
        "{log:?}"
    );
    assert!(
        log.iter()
            .any(|c| c == "rm /data/local/tmp/chrome-command-line"),
        "the command-line file is removed: {log:?}"
    );
    let page = context.new_page().await.unwrap();
    page.goto("data:text/html,<title>Android</title><p>hi</p>")
        .await
        .unwrap();
    assert_eq!(page.title().await.unwrap(), "Android");
    assert_eq!(page.evaluate::<i64>("1 + 1").await.unwrap(), 2);
    let shot = page.screenshot(Default::default()).await.unwrap();
    assert_eq!(&shot[..4], b"\x89PNG");
    context.close().await.unwrap();

    device.close().await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(e) = events.next().await {
            if matches!(e, chromiumoxide::AndroidDeviceEvent::Close) {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(closed.ok(), Some(true), "close event");
}

/// Playwright's command line for Chrome on Android.
#[tokio::test]
async fn android_chrome_command_line() {
    let line = chromiumoxide::android_chrome_command_line(&["--lang=de".to_owned()], "sock");
    assert!(
        line.starts_with(
            "_ --disable-fre --no-default-browser-check --remote-debugging-socket-name=sock "
        ),
        "{line}"
    );
    assert!(
        !line.contains("--disable-sync"),
        "android keeps sync: {line}"
    );
    assert!(line.ends_with(" --lang=de"), "{line}");
}
