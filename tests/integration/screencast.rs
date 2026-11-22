//! `page.screencast`: frames, recording to a file and overlays (API of
//! Playwright 1.63).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chromiumoxide::{Chromium, ContextOptions, LaunchOptions, ScreencastOptions, Viewport};

fn pixel(png_bytes: &[u8], x: u32, y: u32) -> [u8; 3] {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let ch = info.color_type.samples() as u32;
    let i = ((y * info.width + x) * ch) as usize;
    [buf[i], buf[i + 1], buf[i + 2]]
}

#[tokio::test]
async fn screencast_frames_and_recording() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(640, 480)),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = ctx.new_page().await.unwrap();
    page.set_content("<style>body{background:rgb(0,0,255)}</style><h1>cast</h1>")
        .await
        .unwrap();

    let frames = Arc::new(Mutex::new(Vec::new()));
    let sink = frames.clone();
    let cast = page.screencast();
    cast.start(ScreencastOptions::default().on_frame(move |f| {
        sink.lock().unwrap().push(f);
    }))
    .await
    .unwrap();
    let err = cast.start(ScreencastOptions::default()).await.unwrap_err();
    assert_eq!(err.to_string(), "Screencast is already started");
    for _ in 0..50 {
        if !frames.lock().unwrap().is_empty() {
            break;
        }
        page.evaluate::<()>("() => { document.querySelector('h1').textContent = Math.random(); }")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    cast.stop().await.unwrap();
    {
        let frames = frames.lock().unwrap();
        let f = frames.first().expect("a frame");
        assert_eq!(&f.data[..2], &[0xFF, 0xD8], "JPEG");
        assert_eq!((f.viewport_width, f.viewport_height), (640, 480));
        assert!(f.timestamp > 0.0);
    }
    let count = frames.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(frames.lock().unwrap().len(), count, "no frames after stop");

    // Recording to a file (needs ffmpeg, like recordVideo).
    let dir = std::env::temp_dir().join(format!("cox-cast-{}", std::process::id()));
    let path = dir.join("cast.webm");
    cast.start(ScreencastOptions::default().path(&path).size(320, 240))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    cast.stop().await.unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], &[0x1A, 0x45, 0xDF, 0xA3], "webm");
    let _ = std::fs::remove_dir_all(dir);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn screencast_overlays() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(200, 100)),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = ctx.new_page().await.unwrap();
    page.set_content("<style>body{margin:0;background:rgb(255,255,255)}</style>")
        .await
        .unwrap();
    let cast = page.screencast();
    let overlay = cast
        .show_overlay(
            "<div style='position:fixed;inset:0;background:rgb(0,255,0)'></div>",
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        pixel(&page.screenshot().await.unwrap(), 50, 50),
        [0, 255, 0]
    );
    cast.hide_overlays().await.unwrap();
    assert_eq!(
        pixel(&page.screenshot().await.unwrap(), 50, 50),
        [255, 255, 255]
    );
    cast.show_overlays().await.unwrap();
    assert_eq!(
        pixel(&page.screenshot().await.unwrap(), 50, 50),
        [0, 255, 0]
    );

    // Overlays survive navigations of the main frame.
    page.set_content("<style>body{margin:0;background:rgb(255,255,255)}</style><p>new</p>")
        .await
        .unwrap();
    let mut green = false;
    for _ in 0..20 {
        if pixel(&page.screenshot().await.unwrap(), 50, 50) == [0, 255, 0] {
            green = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(green, "overlay re-added after navigation");

    overlay.remove().await.unwrap();
    assert_eq!(
        pixel(&page.screenshot().await.unwrap(), 50, 50),
        [255, 255, 255]
    );

    // A timed overlay goes away by itself; chapters too.
    cast.show_overlay(
        "<div style='position:fixed;inset:0;background:rgb(255,0,0)'></div>",
        Some(Duration::from_millis(100)),
    )
    .await
    .unwrap();
    assert_eq!(
        pixel(&page.screenshot().await.unwrap(), 50, 50),
        [255, 255, 255]
    );
    cast.show_chapter(
        "Chapter",
        Some("about it"),
        Some(Duration::from_millis(100)),
    )
    .await
    .unwrap();
    assert_eq!(
        pixel(&page.screenshot().await.unwrap(), 50, 50),
        [255, 255, 255]
    );
    browser.close().await.unwrap();
}
