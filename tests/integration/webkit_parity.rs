//! Pixel parity with Playwright's WebKit: the same HTML rendered by
//! `chromiumoxide::webkit` and by Playwright's `webkit.launch()`, same
//! WebKit build; every pixel must match. Same cases as the Firefox test,
//! without the pixelated `<img>`: headless WebKit paints that image in some
//! runs and not in others, in Playwright's own runs too (2 of 4 runs of
//! Playwright alone painted it), so no client can match it reliably.

use std::path::{Path, PathBuf};

use chromiumoxide::webkit::WebKitLaunchOptions;
use chromiumoxide::{StyleTag, WebKit};

use crate::firefox_parity::{Case, cases, render_with_playwright_as};
use crate::pixel_parity::{compare, playwright_node_path, shot_options};

#[tokio::test]
async fn webkit_screenshots_are_pixel_identical_to_playwright() {
    let Some(executable) = crate::webkit::webkit_executable() else {
        return;
    };
    let Some(node_path) = playwright_node_path() else {
        assert!(
            std::env::var_os("CHROMIUMOXIDE_REQUIRE_PARITY").is_none(),
            "Playwright is required for the parity test"
        );
        eprintln!("skipping WebKit pixel parity: node + playwright not found");
        return;
    };
    let out: PathBuf = std::env::temp_dir().join(format!("cox-wk-parity-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let cases: Vec<Case> = cases()
        .into_iter()
        .map(|mut c| {
            c.html = unfocused(&without_images(&c.html));
            c
        })
        .collect();
    render_with_playwright_as("webkit", &node_path, &executable, &cases, &out);
    render_with_chromiumoxide(&executable, &cases, &out).await;
    let mut failures = Vec::new();
    for c in &cases {
        for (shot, _) in &c.shots {
            let name = format!("{}-{shot}", c.name);
            if let Err(e) = compare(
                &out.join(format!("{name}.ours.png")),
                &out.join(format!("{name}.png")),
            ) {
                failures.push(format!("{name}: {e}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "WebKit pixel parity failed (images in {}):\n{}",
        out.display(),
        failures.join("\n")
    );
    let _ = std::fs::remove_dir_all(&out);
}

async fn render_with_chromiumoxide(executable: &Path, cases: &[Case], out: &Path) {
    let browser = WebKit::launch(WebKitLaunchOptions {
        executable_path: Some(executable.to_owned()),
        ..Default::default()
    })
    .await
    .unwrap();
    for c in cases {
        let context = browser.new_context(c.context.clone()).await.unwrap();
        let page = context.new_page().await.unwrap();
        page.set_content(&c.html).await.unwrap();
        if let Some(css) = c.style {
            page.add_style_tag(StyleTag::content(css)).await.unwrap();
        }
        if let Some(v) = c.resize {
            page.set_viewport_size(v).await.unwrap();
        }
        for (name, options) in &c.shots {
            let ours = out.join(format!("{}-{name}.ours.png", c.name));
            page.screenshot()
                .options(shot_options(options))
                .path(&ours)
                .await
                .unwrap();
        }
        context.close().await.unwrap();
    }
    browser.close().await.unwrap();
}

/// The HTML without its `<img ...>` tags.
/// Without `autofocus`: whether WebKit paints the focused input's ring in a
/// shot depends on timing, on both sides (Playwright's own shots had it in
/// 3 of 6 runs under load, and focusing from a script raced the same way),
/// so no client can match it reliably.
fn unfocused(html: &str) -> String {
    html.replace(" autofocus ", " ")
}

fn without_images(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("<img") {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find('>') {
            Some(end) => &rest[start + end + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}
