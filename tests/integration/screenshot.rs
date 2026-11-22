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
