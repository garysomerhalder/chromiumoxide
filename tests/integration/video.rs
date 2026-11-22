//! Video recording (size rules from Playwright 1.63; encoded with Playwright's ffmpeg).

use std::time::Duration;

use chromiumoxide::{Chromium, ContextOptions, LaunchOptions, RecordVideo};

fn ffmpeg() -> Option<std::path::PathBuf> {
    let dir = std::fs::read_dir("/opt/pw-browsers").ok()?;
    dir.filter_map(|e| e.ok())
        .map(|e| e.path().join("ffmpeg-linux"))
        .find(|p| p.exists())
}

fn supports_vp9(ffmpeg: &std::path::Path) -> bool {
    let out = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains("libvpx-vp9")
}

#[tokio::test]
async fn records_a_webm_per_page() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("no ffmpeg; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("chromiumoxide-video-{}", std::process::id()));
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            record_video: Some(RecordVideo::new(&dir)),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.set_content(
        "<style>@keyframes c{from{background:red}to{background:blue}} body{animation:c .3s infinite}</style><h1>video</h1>",
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await;
    let video = page.video().expect("recording");
    page.close().await.unwrap();
    let path = video.path().await.unwrap();
    assert!(path.starts_with(&dir), "{path:?}");
    assert_eq!(path.extension().unwrap(), "webm");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], &[0x1a, 0x45, 0xdf, 0xa3], "EBML header");
    assert!(bytes.len() > 1000, "{} bytes", bytes.len());
    let probe = std::process::Command::new(&ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(&path)
        .output()
        .unwrap();
    let info = String::from_utf8_lossy(&probe.stderr);
    // Playwright 1.64 records VP9 (VP8 with an ffmpeg that lacks libvpx-vp9).
    let codec = if supports_vp9(&ffmpeg) { "vp9" } else { "vp8" };
    assert!(info.contains(codec), "{info}");
    assert!(info.contains("800x450"), "{info}");
    let copy = dir.join("copy.webm");
    video.save_as(&copy).await.unwrap();
    assert_eq!(std::fs::read(&copy).unwrap(), bytes);
    video.delete().await.unwrap();
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(&dir);
    browser.close().await.unwrap();
}

/// Playwright 1.64's `recordVideo.fps`.
#[tokio::test]
async fn records_at_the_requested_frame_rate() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("no ffmpeg; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("chromiumoxide-video-fps-{}", std::process::id()));
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions {
            record_video: Some(RecordVideo {
                fps: Some(50),
                ..RecordVideo::new(&dir)
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.set_content("<h1>fps</h1>").await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let video = page.video().expect("recording");
    page.close().await.unwrap();
    let path = video.path().await.unwrap();
    let probe = std::process::Command::new(&ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(&path)
        .output()
        .unwrap();
    let info = String::from_utf8_lossy(&probe.stderr);
    assert!(info.contains("50 fps"), "{info}");
    let err = browser
        .new_context(ContextOptions {
            record_video: Some(RecordVideo {
                fps: Some(0),
                ..RecordVideo::new(&dir)
            }),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("\"recordVideo.fps\" must be a positive number, got 0"),
        "{err}"
    );
    // `screencast.start({ path, fps })`.
    let page = browser.new_page().await.unwrap();
    page.set_content("<h1>cast</h1>").await.unwrap();
    let cast_path = dir.join("cast.webm");
    page.screencast()
        .start(
            chromiumoxide::ScreencastOptions::default()
                .path(&cast_path)
                .fps(30),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    page.screencast().stop().await.unwrap();
    let probe = std::process::Command::new(&ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(&cast_path)
        .output()
        .unwrap();
    let info = String::from_utf8_lossy(&probe.stderr);
    assert!(info.contains("30 fps"), "{info}");
    let _ = std::fs::remove_dir_all(&dir);
    browser.close().await.unwrap();
}
