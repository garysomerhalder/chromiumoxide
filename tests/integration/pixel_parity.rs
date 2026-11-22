//! Proves pixel parity with Playwright: the same HTML is rendered by this crate
//! and by Playwright (same Chromium, same switches) and every pixel must match.
//!
//! Needs Node with the `playwright` package (resolved via `npm root -g`). Set
//! `CHROMIUMOXIDE_REQUIRE_PARITY=1` to fail instead of skipping without it.

use std::path::{Path, PathBuf};
use std::process::Command;

use chromiumoxide::domain::launch::{LaunchSpec, command_line};
use chromiumoxide::{
    Chromium, ColorScheme, ContextOptions, HttpCredentials, LaunchOptions, Rect, StyleTag, Viewport,
};
use serde_json::{Value, json};

use crate::screenshot::decode;

const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAKklEQVQI12P4z8DwnwEIGP7//8/AwMDAxMjIyMDAwMDAxMDAwMDAwMDAwAAAvO8F/qD2QZIAAAAASUVORK5CYII=";

fn rich_html() -> String {
    format!(
        r##"<!doctype html><html><head><style>
body {{ margin: 0; font-family: sans-serif; background: linear-gradient(135deg, #fdfbfb, #ebedee); }}
h1 {{ font: bold 34px serif; color: #223; margin: 12px 16px; }}
p {{ font-size: 15px; line-height: 1.4; margin: 8px 16px; max-width: 520px; }}
code {{ font-family: monospace; background: #eee; padding: 2px 4px; border-radius: 3px; }}
.card {{ margin: 16px; padding: 14px; width: 260px; border-radius: 14px; background: white;
         box-shadow: 0 6px 18px rgba(0,0,0,.25); transform: rotate(-2.5deg); opacity: .93; }}
.grad {{ height: 40px; border-radius: 20px; background: linear-gradient(90deg, #f06, #48f, #0c9); }}
img {{ width: 64px; height: 64px; image-rendering: pixelated; }}
.tall {{ height: 1300px; background: repeating-linear-gradient(0deg, #fff 0 20px, #f4f4ff 20px 40px); }}
@media (prefers-color-scheme: dark) {{ body {{ background: #111; color: #eee; }} }}
</style></head><body>
<h1>Pixel parity — Ünïcødé</h1>
<p>The quick brown fox jumps over the lazy dog. <b>Bold</b>, <i>italic</i>, <code>mono 0123</code>, 12½ × 7 ≈ 88.</p>
<div class="card"><div class="grad"></div><p>Card with shadow, rotation and opacity.</p><img src="{PIXEL}"></div>
<svg width="220" height="90" style="margin-left:16px"><circle cx="45" cy="45" r="38" fill="#e33" stroke="#222" stroke-width="3"/>
<rect x="100" y="12" width="100" height="66" rx="10" fill="rgba(30,120,250,.7)"/><text x="108" y="52" font-size="20" fill="white">SVG</text></svg>
<input value="focused input" autofocus style="margin:16px;font-size:16px">
<div class="tall"></div><p>Bottom of the page.</p>
</body></html>"##
    )
}

fn mobile_html() -> String {
    format!(
        "<meta name=viewport content='width=device-width, initial-scale=1'>{}",
        rich_html().replace("<!doctype html>", "")
    )
}

struct Case {
    name: &'static str,
    context: ContextOptions,
    context_json: Value,
    html: String,
    style: Option<&'static str>,
    resize: Option<Viewport>,
    shots: Vec<(&'static str, Value)>,
}

fn cases() -> Vec<Case> {
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
                is_mobile: true,
                has_touch: true,
                user_agent: Some(iphone_ua.into()),
                locale: Some("en-GB".into()),
                timezone_id: Some("Europe/London".into()),
                ..Default::default()
            },
            context_json: json!({"viewport": {"width": 390, "height": 664}, "screen": {"width": 390, "height": 844},
                "deviceScaleFactor": 3, "isMobile": true, "hasTouch": true, "userAgent": iphone_ua,
                "locale": "en-GB", "timezoneId": "Europe/London"}),
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
                ("css", json!({"scale": "css"})),
            ],
        },
        Case {
            name: "mockup",
            context: ContextOptions {
                viewport: Some(Viewport::new(800, 600)),
                device_scale_factor: Some(2.0),
                color_scheme: Some(ColorScheme::Dark),
                http_credentials: Some(HttpCredentials {
                    username: "u".into(),
                    password: "p".into(),
                    origin: None,
                }),
                ..Default::default()
            },
            context_json: json!({"viewport": {"width": 800, "height": 600}, "deviceScaleFactor": 2, "colorScheme": "dark",
                "httpCredentials": {"username": "u", "password": "p"}}),
            html: rich_html(),
            style: Some(".card { outline: 3px solid #0a0; } h1 { letter-spacing: 1px; }"),
            resize: Some(Viewport::new(1000, 700)),
            shots: vec![
                ("viewport", json!({})),
                ("full", json!({"fullPage": true})),
                (
                    "transparent",
                    json!({"omitBackground": true, "clip": {"x": 0, "y": 0, "width": 500, "height": 300}}),
                ),
            ],
        },
    ]
}

fn playwright_node_path() -> Option<String> {
    let root = Command::new("npm").args(["root", "-g"]).output().ok()?;
    let root = String::from_utf8(root.stdout).ok()?.trim().to_owned();
    let ok = Command::new("node")
        .args(["-e", "require.resolve('playwright')"])
        .env("NODE_PATH", &root)
        .status()
        .ok()?
        .success();
    ok.then_some(root)
}

fn shot_options(v: &Value) -> chromiumoxide::ScreenshotOptions {
    let mut o = chromiumoxide::ScreenshotOptions {
        full_page: v["fullPage"].as_bool().unwrap_or(false),
        omit_background: v["omitBackground"].as_bool().unwrap_or(false),
        ..Default::default()
    };
    if v["scale"] == "css" {
        o.scale = chromiumoxide::ScreenshotScale::Css;
    }
    if let Some(c) = v.get("clip") {
        let f = |k: &str| c[k].as_f64().unwrap();
        o.clip = Some(Rect::new(f("x"), f("y"), f("width"), f("height")));
    }
    o
}

fn compare(ours: &Path, theirs: &Path) -> Result<(), String> {
    let a = decode(&std::fs::read(ours).unwrap());
    let b = decode(&std::fs::read(theirs).map_err(|e| format!("{}: {e}", theirs.display()))?);
    if (a.0, a.1) != (b.0, b.1) {
        return Err(format!(
            "size {}x{} vs Playwright {}x{}",
            a.0, a.1, b.0, b.1
        ));
    }
    let differing =
        a.2.chunks(4)
            .zip(b.2.chunks(4))
            .filter(|(x, y)| x != y)
            .count();
    if differing > 0 {
        return Err(format!("{differing} of {} pixels differ", a.0 * a.1));
    }
    Ok(())
}

#[tokio::test]
async fn screenshots_are_pixel_identical_to_playwright() {
    let Some(node_path) = playwright_node_path() else {
        assert!(
            std::env::var_os("CHROMIUMOXIDE_REQUIRE_PARITY").is_none(),
            "Playwright is required for the parity test"
        );
        eprintln!("skipping pixel parity: node + playwright not found");
        return;
    };
    let found = Chromium::executable_path(&LaunchOptions::default()).unwrap();
    let out: PathBuf = std::env::temp_dir().join(format!("cox-parity-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();

    // Playwright gets exactly our command line (with ignoreDefaultArgs it adds nothing).
    let profile = out.join("playwright-profile");
    let args: Vec<String> = command_line(&LaunchSpec {
        kind: found.kind,
        headless: true,
        args: vec![],
        ignore_default_args: false,
        user_data_dir: profile,
    });

    let cases = cases();
    let job = json!({
        "executablePath": found.executable,
        "args": args,
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
        .env("NODE_PATH", &node_path)
        .status()
        .unwrap();
    assert!(status.success(), "Playwright render failed");

    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let mut failures = Vec::new();
    for c in &cases {
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
            if let Err(e) = compare(&ours, &out.join(format!("{}-{name}.png", c.name))) {
                failures.push(format!("{}-{name}: {e}", c.name));
            }
        }
        context.close().await.unwrap();
    }
    browser.close().await.unwrap();
    assert!(
        failures.is_empty(),
        "pixel parity failed (images in {}):\n{}",
        out.display(),
        failures.join("\n")
    );
    let _ = std::fs::remove_dir_all(&out);
}
