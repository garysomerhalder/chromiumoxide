use chromiumoxide::{
    Chromium, ContextOptions, ImageFormat, LaunchOptions, Rect, ScreenshotScale, Viewport,
};

pub fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        other => panic!("unexpected color type {other:?}"),
    };
    (info.width, info.height, rgba)
}

fn pixel(img: &(u32, u32, Vec<u8>), x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.0 + x) * 4) as usize;
    [img.2[i], img.2[i + 1], img.2[i + 2], img.2[i + 3]]
}

const TALL: &str = "<style>body{margin:0;background:rgb(255,0,0)} #b{position:absolute;top:1500px;left:0;width:50px;height:50px;background:rgb(0,0,255)}</style><div style='height:2000px'></div><div id=b></div>";

#[tokio::test]
async fn viewport_clip_and_full_page_dimensions_follow_device_scale_factor() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(400, 300)),
            device_scale_factor: Some(2.0),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = ctx.new_page().await.unwrap();
    page.set_content(TALL).await.unwrap();

    let shot = decode(&page.screenshot().await.unwrap());
    assert_eq!((shot.0, shot.1), (800, 600));
    assert_eq!(pixel(&shot, 10, 10), [255, 0, 0, 255]);

    let clip = decode(
        &page
            .screenshot()
            .clip(Rect::new(10.0, 20.0, 100.0, 50.0))
            .await
            .unwrap(),
    );
    assert_eq!((clip.0, clip.1), (200, 100));

    let full = decode(&page.screenshot().full_page(true).await.unwrap());
    assert_eq!((full.0, full.1), (800, 4000));
    assert_eq!(
        pixel(&full, 20, 3020),
        [0, 0, 255, 255],
        "element below the fold is captured"
    );

    let css = decode(&page.screenshot().scale(ScreenshotScale::Css).await.unwrap());
    assert_eq!((css.0, css.1), (400, 300));

    // the viewport is restored after a full-page capture
    assert_eq!(
        page.evaluate::<Vec<u32>>("[innerWidth, innerHeight]")
            .await
            .unwrap(),
        vec![400, 300]
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn omit_background_gives_transparency_and_jpeg_works() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content("<p>hi</p>").await.unwrap();
    let opaque = decode(&page.screenshot().await.unwrap());
    assert_eq!(pixel(&opaque, 1279, 719), [255, 255, 255, 255]);
    let transparent = decode(&page.screenshot().omit_background(true).await.unwrap());
    assert_eq!(pixel(&transparent, 1279, 719)[3], 0);
    // background restored afterwards
    let again = decode(&page.screenshot().await.unwrap());
    assert_eq!(pixel(&again, 1279, 719), [255, 255, 255, 255]);

    let jpeg = page
        .screenshot()
        .format(ImageFormat::Jpeg)
        .quality(50)
        .await
        .unwrap();
    assert_eq!(&jpeg[..3], &[0xff, 0xd8, 0xff]);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn screenshot_hides_the_caret_and_writes_to_a_path() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content("<input id=i autofocus>").await.unwrap();
    page.evaluate::<()>("() => document.getElementById('i').focus()")
        .await
        .unwrap();
    let dir = std::env::temp_dir().join(format!("cox-shot-{}", std::process::id()));
    let path = dir.join("a.png");
    let bytes = page.screenshot().path(&path).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    // caret-color restored after capture
    assert_eq!(
        page.evaluate::<String>("document.getElementById('i').style.caretColor")
            .await
            .unwrap(),
        ""
    );
    let _ = std::fs::remove_dir_all(dir);
    let err = page
        .screenshot()
        .clip(Rect::new(0.0, 0.0, 0.0, 10.0))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("clip.width"), "{err}");
    browser.close().await.unwrap();
}

const BOXES: &str = "<style>body{margin:0;background:#fff} div{position:absolute;width:50px;height:50px;background:rgb(255,0,0)} #a{left:0;top:0} #b{left:100px;top:0} #c{left:0;top:100px}</style><div id=a></div><div id=b></div><div id=c class=c></div>";

#[tokio::test]
async fn mask_covers_located_elements_with_a_colour() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(300, 200)),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = ctx.new_page().await.unwrap();
    page.set_content(BOXES).await.unwrap();

    // Default mask colour is Playwright's #F0F.
    let shot = decode(&page.screenshot().mask(&[page.locator("#a")]).await.unwrap());
    assert_eq!(pixel(&shot, 25, 25), [255, 0, 255, 255]);
    assert_eq!(pixel(&shot, 125, 25), [255, 0, 0, 255]);

    // Several locators, a custom colour, non-strict matching.
    let shot = decode(
        &page
            .screenshot()
            .mask(&[page.locator("#b"), page.locator("div.c, #nothing")])
            .mask_color("#00FF00")
            .await
            .unwrap(),
    );
    assert_eq!(pixel(&shot, 25, 25), [255, 0, 0, 255]);
    assert_eq!(pixel(&shot, 125, 25), [0, 255, 0, 255]);
    assert_eq!(pixel(&shot, 25, 125), [0, 255, 0, 255]);

    // The overlay is removed afterwards.
    let shot = decode(&page.screenshot().await.unwrap());
    assert_eq!(pixel(&shot, 125, 25), [255, 0, 0, 255]);

    // Element screenshots take masks too.
    let el = decode(
        &page
            .locator("#a")
            .screenshot_builder()
            .mask(&[page.locator("#a")])
            .mask_color("#0000FF")
            .await
            .unwrap(),
    );
    assert_eq!(pixel(&el, 10, 10), [0, 0, 255, 255]);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn animations_disabled_finishes_finite_and_style_applies_while_capturing() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let ctx = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(300, 100)),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = ctx.new_page().await.unwrap();
    page.set_content(
        "<style>body{margin:0;background:#fff} @keyframes k{from{left:0}to{left:200px}} #m{position:absolute;top:0;left:0;width:50px;height:50px;background:rgb(255,0,0);animation:k 1000s linear forwards}</style><div id=m></div>",
    )
    .await
    .unwrap();

    let running = decode(&page.screenshot().await.unwrap());
    assert_eq!(pixel(&running, 10, 10), [255, 0, 0, 255]);
    assert_eq!(pixel(&running, 225, 10), [255, 255, 255, 255]);

    let disabled = decode(
        &page
            .screenshot()
            .animations(chromiumoxide::Animations::Disabled)
            .await
            .unwrap(),
    );
    assert_eq!(pixel(&disabled, 10, 10), [255, 255, 255, 255]);
    assert_eq!(pixel(&disabled, 225, 10), [255, 0, 0, 255]);

    let styled = decode(
        &page
            .screenshot()
            .style("body { background: rgb(0, 0, 255) !important }")
            .await
            .unwrap(),
    );
    assert_eq!(pixel(&styled, 150, 80), [0, 0, 255, 255]);
    assert_eq!(
        page.evaluate::<String>("getComputedStyle(document.body).backgroundColor")
            .await
            .unwrap(),
        "rgb(255, 255, 255)",
        "the style is removed after capture"
    );
    browser.close().await.unwrap();
}
