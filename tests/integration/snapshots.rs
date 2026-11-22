//! `toHaveScreenshot`: messages recorded from Playwright 1.63.

use std::time::Duration;

use chromiumoxide::{Chromium, ContextOptions, LaunchOptions, UpdateSnapshots, Viewport, expect};

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("cox-snap-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn png_size(path: &std::path::Path) -> (u32, u32) {
    let bytes = std::fs::read(path).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let reader = decoder.read_info().unwrap();
    let info = reader.info();
    (info.width, info.height)
}

#[tokio::test]
async fn page_screenshot_snapshots() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(100, 50)),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = ctx.new_page().await.unwrap();
    let d = dir("page");
    let snap = d.join("a.png");
    page.set_content("<style>body{margin:0;background:red}</style>")
        .await
        .unwrap();

    // Missing: written, and the assertion fails like Playwright's.
    let err = expect(&page).to_have_screenshot(&snap).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "A snapshot doesn't exist at {}, writing actual.",
            snap.display()
        )
    );
    assert_eq!(png_size(&snap), (100, 50));

    // Same page: passes.
    expect(&page).to_have_screenshot(&snap).await.unwrap();
    expect(&page)
        .not()
        .to_have_screenshot(&d.join("missing.png"))
        .await
        .unwrap_err();

    // Different: fails with the pixel count, and writes the artifacts.
    page.set_content("<style>body{margin:0;background:#00f}</style>")
        .await
        .unwrap();
    let out = d.join("results");
    let err = expect(&page)
        .to_have_screenshot(&snap)
        .output_dir(&out)
        .timeout(Duration::from_secs(2))
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "expect(page).toHaveScreenshot(expected) failed\n\n  5000 pixels (ratio 1.00 of all image pixels) are different.\n\n  Snapshot: {}",
            snap.display()
        )
    );
    for f in ["a-actual.png", "a-expected.png", "a-diff.png"] {
        assert!(out.join(f).exists(), "{f} written");
    }
    expect(&page).not().to_have_screenshot(&snap).await.unwrap();
    expect(&page)
        .to_have_screenshot(&snap)
        .max_diff_pixels(5000)
        .await
        .unwrap();
    expect(&page)
        .to_have_screenshot(&snap)
        .max_diff_pixel_ratio(1.0)
        .await
        .unwrap();
    // Red and blue are far apart, so even the loosest threshold reports them.
    expect(&page)
        .to_have_screenshot(&snap)
        .threshold(0.5)
        .timeout(Duration::from_secs(1))
        .await
        .unwrap_err();
    // A mask over everything, same colour as the snapshot: matches.
    expect(&page)
        .to_have_screenshot(&snap)
        .mask(&[page.locator("html")])
        .mask_color("rgb(255, 0, 0)")
        .await
        .unwrap();

    // Update: the snapshot is rewritten.
    expect(&page)
        .to_have_screenshot(&snap)
        .update(UpdateSnapshots::All)
        .await
        .unwrap();
    expect(&page).to_have_screenshot(&snap).await.unwrap();
    let err = expect(&page)
        .not()
        .to_have_screenshot(&snap)
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Screenshot comparison failed:\n\n  Expected result should be different from the actual one."
    );

    // UpdateSnapshots::None does not write a missing snapshot.
    let none = d.join("none.png");
    let err = expect(&page)
        .to_have_screenshot(&none)
        .update(UpdateSnapshots::None)
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "A snapshot doesn't exist at {}.",
            none.display()
        )
    );
    assert!(!none.exists());

    let err = expect(&page)
        .to_have_screenshot(&d.join("a.jpg"))
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Screenshot name \"a.jpg\" must have a '.png' or '.webp' extension"
    );
    let _ = std::fs::remove_dir_all(d);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn locator_screenshot_snapshots_and_animations() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    let d = dir("locator");
    let snap = d.join("d.png");
    page.set_content("<style>body{margin:0}</style><div id=d style=\"width:20px;height:20px;background:blue\"></div>")
        .await
        .unwrap();
    let d_loc = page.locator("#d");
    let _ = expect(&d_loc).to_have_screenshot(&snap).await;
    assert_eq!(png_size(&snap), (20, 20));
    expect(&d_loc).to_have_screenshot(&snap).await.unwrap();

    page.set_content("<style>body{margin:0}</style><div id=d style=\"width:30px;height:20px;background:red\"></div>")
        .await
        .unwrap();
    let err = expect(&d_loc).to_have_screenshot(&snap).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "expect(locator).toHaveScreenshot(expected) failed\n\nLocator: locator('#d')\n  Expected an image 20px by 20px, received 30px by 20px. 600 pixels (ratio 1.00 of all image pixels) are different.\n\n  Snapshot: {}",
            snap.display()
        )
    );

    // Animations are disabled by default, so a running animation is stable.
    let anim = d.join("anim.png");
    page.set_content("<style>body{margin:0} @keyframes k{from{left:0}to{left:200px}} #m{position:absolute;width:20px;height:20px;background:red;animation:k 1000s linear forwards}</style><div id=m></div>")
        .await
        .unwrap();
    let _ = expect(&page).to_have_screenshot(&anim).await;
    expect(&page).to_have_screenshot(&anim).await.unwrap();
    let _ = std::fs::remove_dir_all(d);
    browser.close().await.unwrap();
}
