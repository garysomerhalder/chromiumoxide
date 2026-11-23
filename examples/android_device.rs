//! Drives a real Android device or emulator through the local adb server
//! (Playwright's `_android`): lists devices, runs a shell command, pushes a
//! file, takes a screenshot, launches Chrome and loads a page.
//!
//! `cargo run --example android_device`

use std::time::Duration;

use chromiumoxide::{Android, AndroidDevicesOptions, AndroidLaunchOptions};

#[tokio::main]
async fn main() -> chromiumoxide::Result<()> {
    let devices = Android::devices(AndroidDevicesOptions::default()).await?;
    let Some(device) = devices.first() else {
        eprintln!("no Android device: start an emulator or connect a phone");
        std::process::exit(2);
    };
    println!("device {} ({})", device.serial(), device.model());

    let out = device
        .shell("echo hello from $(getprop ro.build.version.release)")
        .await?;
    println!("shell: {}", String::from_utf8_lossy(&out).trim());

    device
        .push(b"pushed by chromiumoxide", "/data/local/tmp/cox.txt", 0o644)
        .await?;
    let back = device.shell("cat /data/local/tmp/cox.txt").await?;
    println!("push + cat: {}", String::from_utf8_lossy(&back));
    device.shell("rm /data/local/tmp/cox.txt").await?;

    let png = device.screenshot().await?;
    println!(
        "screenshot: {} bytes, PNG: {}",
        png.len(),
        png.starts_with(b"\x89PNG")
    );

    let context = device
        .launch_browser(AndroidLaunchOptions {
            timeout: Duration::from_secs(60),
            ..Default::default()
        })
        .await?;
    let page = context.new_page().await?;
    page.goto("data:text/html,<title>chromiumoxide on Android</title><h1>Hello Android</h1>")
        .await?;
    println!("title: {}", page.title().await?);
    println!(
        "h1: {:?}",
        page.locator("h1").text_content().await?.unwrap_or_default()
    );
    println!(
        "user agent: {}",
        page.evaluate::<String>("navigator.userAgent").await?
    );
    let shot = page.screenshot().await?;
    std::fs::write("android-page.png", &shot).expect("write android-page.png");
    println!("page screenshot: {} bytes -> android-page.png", shot.len());
    context.close().await?;
    device.close().await?;
    println!("OK");
    Ok(())
}
