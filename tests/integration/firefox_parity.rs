//! Pixel parity with Playwright's Firefox: the same HTML rendered by
//! `chromiumoxide::firefox` (WebDriver BiDi) and by Playwright's
//! `firefox.launch()` (Juggler), same Firefox build; every pixel must match.
//!
//! The cases are the Chromium parity cases without what Firefox has no
//! BiDi command for (isMobile, hasTouch, colorScheme); see PARITY.

use std::path::{Path, PathBuf};
use std::process::Command;

use chromiumoxide::firefox::FirefoxLaunchOptions;
use chromiumoxide::{ContextOptions, Firefox, StyleTag, Viewport};
use serde_json::{Value, json};

use crate::pixel_parity::{compare, mobile_html, playwright_node_path, rich_html, shot_options};

pub(crate) struct Case {
    pub(crate) name: &'static str,
    pub(crate) context: ContextOptions,
    pub(crate) context_json: Value,
    pub(crate) html: String,
    pub(crate) style: Option<&'static str>,
    pub(crate) resize: Option<Viewport>,
    pub(crate) shots: Vec<(&'static str, Value)>,
}

pub(crate) fn cases() -> Vec<Case> {
    let iphone_ua = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";
    vec![
        Case {
            name: "desktop",
            context: ContextOptions::default(),
            context_json: json!({}),
            html: rich_html(),
            style: None,
            resize: None,
            shots: vec![
                ("viewport", json!({})),
                (
                    "clip",
                    json!({"clip": {"x": 13.5, "y": 7.25, "width": 300, "height": 200}}),
                ),
                ("full", json!({"fullPage": true})),
            ],
        },
        Case {
            name: "mobile",
            context: ContextOptions {
                viewport: Some(Viewport::new(390, 664)),
                screen: Some(Viewport::new(390, 844)),
                device_scale_factor: Some(3.0),
                user_agent: Some(iphone_ua.into()),
                locale: Some("en-GB".into()),
                timezone_id: Some("Europe/London".into()),
                ..Default::default()
            },
            context_json: json!({"viewport": {"width": 390, "height": 664}, "screen": {"width": 390, "height": 844},
                "deviceScaleFactor": 3, "userAgent": iphone_ua, "locale": "en-GB", "timezoneId": "Europe/London"}),
            html: mobile_html(),
            style: None,
            resize: None,
            shots: vec![
                ("viewport", json!({})),
                (
                    "clip",
                    json!({"clip": {"x": 0, "y": 40, "width": 390, "height": 300}}),
                ),
                ("full", json!({"fullPage": true})),
            ],
        },
        Case {
            name: "mockup",
            context: ContextOptions {
                viewport: Some(Viewport::new(800, 600)),
                device_scale_factor: Some(2.0),
                ..Default::default()
            },
            context_json: json!({"viewport": {"width": 800, "height": 600}, "deviceScaleFactor": 2}),
            html: rich_html(),
            style: Some(".card { outline: 3px solid #0a0; } h1 { letter-spacing: 1px; }"),
            resize: Some(Viewport::new(1000, 700)),
            shots: vec![("viewport", json!({})), ("full", json!({"fullPage": true}))],
        },
    ]
}

#[tokio::test]
async fn firefox_screenshots_are_pixel_identical_to_playwright() {
    let Some(executable) = crate::firefox::firefox_executable() else {
        return;
    };
    let Some(node_path) = playwright_node_path() else {
        assert!(
            std::env::var_os("CHROMIUMOXIDE_REQUIRE_PARITY").is_none(),
            "Playwright is required for the parity test"
        );
        eprintln!("skipping Firefox pixel parity: node + playwright not found");
        return;
    };
    let out: PathBuf = std::env::temp_dir().join(format!("cox-ff-parity-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let cases = cases();
    render_with_playwright(&node_path, &executable, &cases, &out);
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
        "Firefox pixel parity failed (images in {}):\n{}",
        out.display(),
        failures.join("\n")
    );
    let _ = std::fs::remove_dir_all(&out);
}

fn render_with_playwright(node_path: &str, executable: &Path, cases: &[Case], out: &Path) {
    render_with_playwright_as("firefox", node_path, executable, cases, out);
}

/// Renders the cases with Playwright's `browser` (`firefox` or `webkit`).
pub(crate) fn render_with_playwright_as(
    browser: &str,
    node_path: &str,
    executable: &Path,
    cases: &[Case],
    out: &Path,
) {
    let job = json!({
        "browser": browser,
        "executablePath": executable,
        "args": [],
        "outDir": out,
        "cases": cases.iter().map(|c| json!({
            "name": c.name, "context": c.context_json, "html": c.html, "style": c.style,
            "resize": c.resize.map(|v| json!({"width": v.width, "height": v.height})),
            "shots": c.shots.iter().map(|(n, o)| json!({"name": n, "options": o})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    let job_path = out.join("job.json");
    std::fs::write(&job_path, job.to_string()).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity/render.js");
    let status = Command::new("node")
        .arg(&script)
        .arg(&job_path)
        .env("NODE_PATH", node_path)
        .status()
        .unwrap();
    assert!(status.success(), "Playwright's {browser} render failed");
}

async fn render_with_chromiumoxide(executable: &Path, cases: &[Case], out: &Path) {
    let browser = Firefox::launch(FirefoxLaunchOptions {
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
