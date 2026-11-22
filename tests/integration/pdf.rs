//! page.pdf() (characteristics recorded from Playwright 1.63 output).

use chromiumoxide::{Chromium, LaunchOptions, PdfOptions};

fn pages(pdf: &[u8]) -> usize {
    let text = String::from_utf8_lossy(pdf);
    regex::Regex::new(r"/Type\s*/Page\b")
        .unwrap()
        .find_iter(&text)
        .count()
}

fn media_box(pdf: &[u8]) -> String {
    let text = String::from_utf8_lossy(pdf);
    regex::Regex::new(r"MediaBox\s*\[[^\]]*\]")
        .unwrap()
        .find(&text)
        .map(|m| m.as_str().to_owned())
        .unwrap_or_default()
}

#[tokio::test]
async fn pdf_like_playwright() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content("<h1>One</h1><div style=\"page-break-before:always\">Two</div>")
        .await
        .unwrap();
    let pdf = page.pdf().await.unwrap();
    assert!(pdf.starts_with(b"%PDF-1.4"));
    assert_eq!(pages(&pdf), 2);
    assert_eq!(media_box(&pdf), "MediaBox [0 0 612 792]");
    let a4 = page
        .pdf()
        .options(PdfOptions {
            format: Some("A4".into()),
            landscape: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(media_box(&a4), "MediaBox [0 0 842.88 595.91998]");
    let second = page
        .pdf()
        .options(PdfOptions {
            page_ranges: "2".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(pages(&second), 1);
    let bad = page
        .pdf()
        .options(PdfOptions {
            format: Some("B5".into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(bad.to_string().contains("Unknown paper format: B5"));
    browser.close().await.unwrap();
}
