use chromiumoxide::*;
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

#[tokio::main]
async fn main() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    for run in 0..100 {
        let ctx = browser
            .new_context(ContextOptions {
                viewport: Some(Viewport::new(800, 600)),
                device_scale_factor: if std::env::var("DSF1").is_ok() {
                    None
                } else {
                    Some(2.0)
                },
                color_scheme: if std::env::var("LIGHT").is_ok() {
                    None
                } else {
                    Some(ColorScheme::Dark)
                },
                http_credentials: if std::env::var("AUTH").is_ok() {
                    Some(HttpCredentials {
                        username: "u".into(),
                        password: "p".into(),
                        origin: None,
                    })
                } else {
                    None
                },
                ..Default::default()
            })
            .await
            .unwrap();
        let page = ctx.new_page().await.unwrap();
        if std::env::var("FRESH").is_ok() {
            page.goto("about:blank").await.unwrap();
        }
        if let Ok(ms) = std::env::var("SLEEP") {
            tokio::time::sleep(std::time::Duration::from_millis(ms.parse().unwrap())).await;
        }
        page.set_content(&if std::env::var("NOAF").is_ok() {
            rich_html().replace(" autofocus", "")
        } else {
            rich_html()
        })
        .await
        .unwrap();
        let m = "() => [...document.querySelectorAll('body *')].map(e => { const r = e.getBoundingClientRect(); return e.tagName + ':' + [r.x, r.y, r.width, r.height].map(v => Math.round(v*100)/100).join(',') + ':' + getComputedStyle(e).fontFamily; }).join(' | ')";
        let a: serde_json::Value = page.evaluate(m).await.unwrap();
        page.add_style_tag(StyleTag::content(
            ".card { outline: 3px solid #0a0; } h1 { letter-spacing: 1px; }",
        ))
        .await
        .unwrap();
        println!("run {run}: {a}");
        ctx.close().await.unwrap();
    }
    browser.close().await.unwrap();
}
