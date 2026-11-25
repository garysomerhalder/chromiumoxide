//! The Playwright API through the extension: a Chromium that was not started
//! for automation, reached only through `extension/`. One test per area.

use std::path::PathBuf;
use std::time::Duration;

use chromiumoxide::{Browser, BrowserContext, Chromium, ExtensionLaunchOptions, NewCookie, Page};
use serde_json::{Value, json};

use crate::server::TestServer;

/// A Chromium started as an ordinary browser (no debugging port, no
/// automation switches) with the extension, driven only through it.
pub(crate) struct Ext {
    pub browser: Browser,
    pub ctx: BrowserContext,
    profile: PathBuf,
}

impl Ext {
    pub async fn page(&self) -> Page {
        self.ctx.new_page().await.unwrap()
    }
    pub async fn close(self) {
        self.browser.close().await.unwrap();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

pub(crate) async fn connect() -> Option<Ext> {
    let extension = crate::extension::built("chrome")?;
    let chromium = crate::extension::full_chromium()?;
    let profile = std::env::temp_dir().join(format!(
        "cox-extapi-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    let browser = Chromium::launch_with_extension(ExtensionLaunchOptions {
        executable_path: Some(chromium),
        extension_path: extension,
        user_data_dir: profile.clone(),
        // Containers run tests as root, where Chromium needs this.
        args: if cfg!(target_os = "linux") {
            vec!["--no-sandbox".into()]
        } else {
            Vec::new()
        },
        ..Default::default()
    })
    .await
    .unwrap();
    let ctx = browser.contexts().into_iter().next().unwrap();
    Some(Ext {
        browser,
        ctx,
        profile,
    })
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[tokio::test]
async fn extension_navigation_and_waits() {
    let Some(ext) = connect().await else { return };
    let server = TestServer::start().await;
    server.route("/a", "text/html", "<title>A</title><a href=/b>b</a>", 0);
    server.route("/b", "text/html", "<title>B</title><p>b</p>", 0);
    let page = ext.page().await;
    page.goto(&server.url("/a")).await.unwrap();
    assert_eq!(page.title().await.unwrap(), "A");
    page.click("a").await.unwrap();
    page.wait_for_url("**/b").await.unwrap();
    assert_eq!(page.title().await.unwrap(), "B");
    page.go_back().await.unwrap();
    assert_eq!(page.title().await.unwrap(), "A");
    page.go_forward().await.unwrap();
    page.reload().await.unwrap();
    assert!(page.content().await.unwrap().contains("<p>b</p>"));
    page.set_content("<h1>set</h1>").await.unwrap();
    assert_eq!(page.locator("h1").inner_text().await.unwrap(), "set");
    page.evaluate::<()>("void setTimeout(() => window.ready = true, 50)")
        .await
        .unwrap();
    page.wait_for_function("() => window.ready === true")
        .await
        .unwrap();
    ext.close().await;
}

#[tokio::test]
async fn extension_locators_input_and_assertions() {
    let Some(ext) = connect().await else { return };
    let page = ext.page().await;
    page.set_content(
        r#"<label>Name <input id=n></label>
        <input type=checkbox id=c><select id=s><option>a</option><option>b</option></select>
        <button onclick="out.textContent = n.value + '|' + c.checked + '|' + s.value">Go</button>
        <div id=d ondblclick="this.textContent='dbl'">x</div>
        <div id=h onmouseover="this.textContent='hovered'">h</div>
        <textarea id=t></textarea><p id=out></p>"#,
    )
    .await
    .unwrap();
    page.get_by_label("Name", false).fill("Gary").await.unwrap();
    page.locator("#c").check().await.unwrap();
    page.locator("#s").select_option(&["b"]).await.unwrap();
    page.get_by_role("button", chromiumoxide::RoleOptions::default())
        .click()
        .await
        .unwrap();
    chromiumoxide::expect(&page.locator("#out"))
        .to_have_text("Gary|true|b")
        .await
        .unwrap();
    page.locator("#d").dblclick().await.unwrap();
    assert_eq!(page.locator("#d").inner_text().await.unwrap(), "dbl");
    page.locator("#h").hover().await.unwrap();
    assert_eq!(page.locator("#h").inner_text().await.unwrap(), "hovered");
    page.locator("#t").press_sequentially("abc").await.unwrap();
    page.locator("#t").press("Backspace").await.unwrap();
    assert_eq!(page.locator("#t").input_value().await.unwrap(), "ab");
    page.keyboard().type_text("Z").await.unwrap();
    assert_eq!(page.locator("#t").input_value().await.unwrap(), "abZ");
    ext.close().await;
}

#[tokio::test]
async fn extension_scripts_handles_and_bindings() {
    let Some(ext) = connect().await else { return };
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p id=p>hi</p>", 0);
    let page = ext.page().await;
    page.add_init_script("window.__early = 7").await.unwrap();
    page.expose_function("twice", |args: Vec<Value>| async move {
        Ok(json!(args[0].as_i64().unwrap() * 2))
    })
    .await
    .unwrap();
    page.goto(&server.url("/")).await.unwrap();
    assert_eq!(page.evaluate::<i64>("window.__early").await.unwrap(), 7);
    assert_eq!(page.evaluate::<i64>("twice(21)").await.unwrap(), 42);
    let handle = page
        .evaluate_handle("({ text: document.querySelector('#p').textContent })")
        .await
        .unwrap();
    assert_eq!(
        handle.json_value::<Value>().await.unwrap(),
        json!({"text": "hi"})
    );
    page.add_style_tag(chromiumoxide::StyleTag::content(
        "p { color: rgb(1, 2, 3) }",
    ))
    .await
    .unwrap();
    assert_eq!(
        page.evaluate::<String>("getComputedStyle(p).color")
            .await
            .unwrap(),
        "rgb(1, 2, 3)"
    );
    ext.close().await;
}

#[tokio::test]
async fn extension_frames_including_cross_origin() {
    let Some(ext) = connect().await else { return };
    let a = TestServer::start().await;
    let b = TestServer::start().await;
    b.route(
        "/inner",
        "text/html",
        "<button onclick=\"this.textContent='in'\">x</button>",
        0,
    );
    // 127.0.0.1 and localhost are different sites: an out-of-process iframe.
    let inner = b.url("/inner").replace("127.0.0.1", "localhost");
    a.route(
        "/",
        "text/html",
        format!("<iframe src='{inner}'></iframe>").into_bytes(),
        0,
    );
    let page = ext.page().await;
    page.goto(&a.url("/")).await.unwrap();
    let button = page.frame_locator("iframe").locator("button");
    button.click().await.unwrap();
    assert_eq!(button.inner_text().await.unwrap(), "in");
    ext.close().await;
}

#[tokio::test]
async fn extension_events_dialogs_popups_and_websockets() {
    let Some(ext) = connect().await else { return };
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>x</p>", 0);
    server.route("/pop", "text/html", "<title>Pop</title>", 0);
    let page = ext.page().await;
    page.goto(&server.url("/")).await.unwrap();
    let msg = page
        .wait_for_console_message(|m| m.text() == "hello")
        .run(page.evaluate::<()>("console.log('hello')"))
        .await
        .unwrap();
    assert_eq!(msg.kind(), "log");
    page.on_dialog(|d| async move {
        d.accept(Some("yes")).await.unwrap();
    });
    assert_eq!(
        page.evaluate::<String>("prompt('q?')").await.unwrap(),
        "yes"
    );
    let popup = page
        .wait_for_popup()
        .run(page.evaluate::<()>(&format!("void window.open('{}')", server.url("/pop"))))
        .await
        .unwrap();
    popup
        .wait_for_load_state(chromiumoxide::WaitUntil::Load, None)
        .await
        .unwrap();
    assert_eq!(popup.title().await.unwrap(), "Pop");
    let response = page
        .wait_for_response(server.url("/").as_str())
        .run(page.reload())
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    ext.close().await;
}

#[tokio::test]
async fn extension_routes_headers_and_offline() {
    let Some(ext) = connect().await else { return };
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>real</p>", 0);
    let page = ext.page().await;
    page.route("**/fake", |r: chromiumoxide::Route| async move {
        r.fulfill().body("<p>faked</p>").await.unwrap();
    })
    .await
    .unwrap();
    page.goto(&server.url("/fake")).await.unwrap();
    assert_eq!(page.locator("p").inner_text().await.unwrap(), "faked");
    page.set_extra_http_headers([("x-test", "1")])
        .await
        .unwrap();
    page.goto(&server.url("/")).await.unwrap();
    assert_eq!(page.locator("p").inner_text().await.unwrap(), "real");
    ext.ctx.set_offline(true).await.unwrap();
    assert!(page.goto(&server.url("/")).await.is_err());
    ext.ctx.set_offline(false).await.unwrap();
    ext.close().await;
}

#[tokio::test]
async fn extension_cookies_storage_and_permissions() {
    let Some(ext) = connect().await else { return };
    let server = TestServer::start().await;
    server.route("/", "text/html", "<p>x</p>", 0);
    let page = ext.page().await;
    page.goto(&server.url("/")).await.unwrap();
    ext.ctx
        .add_cookies(&[NewCookie::new("k", "v").url(&server.url("/"))])
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<String>("document.cookie").await.unwrap(),
        "k=v"
    );
    let cookies = ext.ctx.cookies().await.unwrap();
    assert!(cookies.iter().any(|c| c.name == "k" && c.value == "v"));
    page.evaluate::<()>("localStorage.setItem('a', '1')")
        .await
        .unwrap();
    let state = ext.ctx.storage_state().await.unwrap();
    assert!(state.cookies.iter().any(|c| c.name == "k"));
    ext.ctx.clear_cookies().await.unwrap();
    assert_eq!(
        page.evaluate::<String>("document.cookie").await.unwrap(),
        ""
    );
    ext.ctx
        .grant_permissions(&["geolocation"], None)
        .await
        .unwrap();
    ext.ctx
        .set_geolocation(Some(chromiumoxide::Geolocation {
            latitude: 10.0,
            longitude: 20.0,
            accuracy: 1.0,
        }))
        .await
        .unwrap();
    let lat = page
        .evaluate::<f64>(
            "new Promise(r => navigator.geolocation.getCurrentPosition(p => r(p.coords.latitude)))",
        )
        .await
        .unwrap();
    assert_eq!(lat, 10.0);
    ext.close().await;
}

#[tokio::test]
async fn extension_emulation_screenshots_pdf_and_aria() {
    let Some(ext) = connect().await else { return };
    let page = ext.page().await;
    page.set_content("<h1>Title</h1><button>OK</button>")
        .await
        .unwrap();
    page.set_viewport_size(chromiumoxide::Viewport::new(640, 480))
        .await
        .unwrap();
    assert_eq!(page.evaluate::<i64>("innerWidth").await.unwrap(), 640);
    page.emulate_media(chromiumoxide::EmulateMedia {
        color_scheme: Some(chromiumoxide::ColorScheme::Dark),
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(
        page.evaluate::<bool>("matchMedia('(prefers-color-scheme: dark)').matches")
            .await
            .unwrap()
    );
    let full = page.screenshot().full_page(true).await.unwrap();
    assert!(full.starts_with(b"\x89PNG"));
    let shot = page.locator("button").screenshot().await.unwrap();
    assert!(shot.starts_with(b"\x89PNG"));
    let pdf = page.pdf().await.unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    let aria = page.locator("body").aria_snapshot().await.unwrap();
    assert!(aria.contains("heading \"Title\""), "{aria}");
    ext.close().await;
}

#[tokio::test]
async fn extension_clock_tracing_and_downloads() {
    let Some(ext) = connect().await else { return };
    let server = TestServer::start().await;
    server.route("/", "text/html", "<a id=d href=/file download>get</a>", 0);
    server.route("/file", "application/octet-stream", "payload", 0);
    let page = ext.page().await;
    page.clock().set_fixed_time(1_000_000u64).await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    assert_eq!(page.evaluate::<u64>("Date.now()").await.unwrap(), 1_000_000);

    ext.ctx
        .tracing()
        .start(chromiumoxide::TracingStart {
            screenshots: true,
            snapshots: true,
            ..Default::default()
        })
        .await
        .unwrap();
    page.click("body").await.unwrap();
    let trace = std::env::temp_dir().join(format!("cox-ext-trace-{}.zip", rand_suffix()));
    ext.ctx.tracing().stop(Some(&trace)).await.unwrap();
    assert!(std::fs::metadata(&trace).unwrap().len() > 100);
    let _ = std::fs::remove_file(&trace);

    let download = tokio::time::timeout(
        Duration::from_secs(20),
        page.wait_for_download().run(page.click("#d")),
    )
    .await
    .expect("a download within 20 s")
    .unwrap();
    let saved = std::env::temp_dir().join(format!("cox-ext-dl-{}", rand_suffix()));
    download.save_as(&saved).await.unwrap();
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), "payload");
    let _ = std::fs::remove_file(&saved);
    ext.close().await;
}
