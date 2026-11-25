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
    connect_with(|_| {}).await
}

/// [`connect`], with launch options changed by `edit`.
pub(crate) async fn connect_with(edit: impl FnOnce(&mut ExtensionLaunchOptions)) -> Option<Ext> {
    let extension = crate::extension::built("chrome")?;
    let chromium = crate::extension::full_chromium()?;
    let profile = std::env::temp_dir().join(format!(
        "cox-extapi-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    let mut options = ExtensionLaunchOptions {
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
    };
    edit(&mut options);
    let browser = Chromium::launch_with_extension(options).await.unwrap();
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

#[tokio::test]
async fn extension_uploads_file_chooser_and_drag_and_drop() {
    let Some(ext) = connect().await else { return };
    let page = ext.page().await;
    page.set_content("<input type=file id=f><input type=file id=g>")
        .await
        .unwrap();
    let file = std::env::temp_dir().join(format!("cox-ext-up-{}.txt", rand_suffix()));
    std::fs::write(&file, "upload").unwrap();
    page.locator("#f")
        .set_input_files(std::slice::from_ref(&file))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<String>("f.files[0].name").await.unwrap(),
        file.file_name().unwrap().to_string_lossy()
    );
    let chooser = page
        .wait_for_file_chooser()
        .run(page.click("#g"))
        .await
        .unwrap();
    chooser
        .set_files(std::slice::from_ref(&file))
        .await
        .unwrap();
    assert_eq!(page.evaluate::<i64>("g.files.length").await.unwrap(), 1);
    let _ = std::fs::remove_file(&file);

    page.set_content(
        r#"<style>div{width:100px;height:100px;margin:20px}</style>
        <div id=src draggable=true style="background:red">drag</div><div id=dst style="background:blue"></div>
        <script>window.log=[];
        src.addEventListener('dragstart', e => { e.dataTransfer.setData('text/plain', 'payload'); });
        dst.addEventListener('dragover', e => e.preventDefault());
        dst.addEventListener('drop', e => { e.preventDefault(); log.push('drop:' + e.dataTransfer.getData('text/plain')); });
        </script>"#,
    )
    .await
    .unwrap();
    page.locator("#src")
        .drag_to(&page.locator("#dst"))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<Vec<String>>("window.log").await.unwrap(),
        ["drop:payload"]
    );
    ext.close().await;
}

#[tokio::test]
async fn extension_workers_websockets_screencast_and_request_api() {
    let Some(ext) = connect().await else { return };
    let server = TestServer::start().await;
    server.route("/", "text/html", "<h1>x</h1>", 0);
    server.route("/api", "application/json", r#"{"ok":true}"#, 0);
    let page = ext.page().await;
    page.goto(&server.url("/")).await.unwrap();

    let worker = page
        .wait_for_worker()
        .run(page.evaluate::<()>(
            "void (window.w = new Worker(URL.createObjectURL(new Blob(['self.x = 7'], {type: 'application/javascript'}))))",
        ))
        .await
        .unwrap();
    assert!(worker.url().starts_with("blob:"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(worker.evaluate::<i64>("self.x").await.unwrap(), 7);

    let port = crate::workers::ws_server().await;
    let ws = page
        .wait_for_websocket()
        .run(page.evaluate::<()>(&format!(
            "void (window.sock = new WebSocket('ws://127.0.0.1:{port}/sock'), sock.onopen = () => sock.send('hello'))"
        )))
        .await
        .unwrap();
    assert!(ws.url().ends_with("/sock"));

    let frames = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let sink = frames.clone();
    let cast = page.screencast();
    cast.start(
        chromiumoxide::ScreencastOptions::default().on_frame(move |_| {
            *sink.lock().unwrap() += 1;
        }),
    )
    .await
    .unwrap();
    for _ in 0..50 {
        if *frames.lock().unwrap() > 0 {
            break;
        }
        page.evaluate::<()>("void (document.querySelector('h1').textContent = Math.random())")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    cast.stop().await.unwrap();
    assert!(*frames.lock().unwrap() > 0, "screencast frames");

    let r = ext.ctx.request().get(&server.url("/api")).await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"ok": true}));
    ext.close().await;
}

#[tokio::test]
async fn extension_replays_a_har() {
    let Some(ext) = connect().await else { return };
    let server = TestServer::start().await;
    server.route("/page", "text/html", "<p>recorded</p>", 0);
    // Recorded with an ordinary launch, replayed through the extension.
    let har = std::env::temp_dir().join(format!("cox-ext-{}.har", rand_suffix()));
    let recorder = Chromium::launch(chromiumoxide::LaunchOptions::default())
        .await
        .unwrap();
    let ctx = recorder
        .new_context(chromiumoxide::ContextOptions {
            record_har: Some(chromiumoxide::RecordHar::new(&har)),
            ..Default::default()
        })
        .await
        .unwrap();
    let p = ctx.new_page().await.unwrap();
    p.goto(&server.url("/page")).await.unwrap();
    ctx.close().await.unwrap();
    recorder.close().await.unwrap();

    let page = ext.page().await;
    page.route_from_har(&har).await.unwrap();
    server.route("/page", "text/html", "<p>changed</p>", 0);
    page.goto(&server.url("/page")).await.unwrap();
    assert_eq!(page.locator("p").inner_text().await.unwrap(), "recorded");
    let _ = std::fs::remove_file(&har);
    ext.close().await;
}

#[tokio::test]
async fn extension_opens_data_urls() {
    let Some(ext) = connect().await else { return };
    let page = ext.page().await;
    // Chrome refuses data: navigations from extensions; the library shows
    // the content in the tab instead.
    page.goto("data:text/html,<title>Inline</title><p>inline%20page</p>")
        .await
        .unwrap();
    assert_eq!(page.title().await.unwrap(), "Inline");
    assert_eq!(page.locator("p").inner_text().await.unwrap(), "inline page");
    page.goto("data:text/html;base64,PHA+YmFzZTY0PC9wPg==")
        .await
        .unwrap();
    assert_eq!(page.locator("p").inner_text().await.unwrap(), "base64");
    ext.close().await;
}

#[tokio::test]
async fn extension_records_video_and_har() {
    let dir = std::env::temp_dir().join(format!("cox-ext-rec-{}", rand_suffix()));
    let har = dir.join("traffic.har");
    let (video_dir, har_path) = (dir.clone(), har.clone());
    let Some(ext) = connect_with(move |o| {
        o.record_video = Some(chromiumoxide::RecordVideo::new(&video_dir));
        o.record_har = Some(chromiumoxide::RecordHar::new(&har_path));
    })
    .await
    else {
        return;
    };
    let server = TestServer::start().await;
    server.route("/", "text/html", "<h1>recorded</h1>", 0);
    let page = ext.page().await;
    page.goto(&server.url("/")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let video = page
        .video()
        .expect("a video for pages through the extension");
    page.close().await.unwrap();
    let path = video.path().await.unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > 0, "{path:?}");
    ext.close().await;
    let log: Value = serde_json::from_slice(&std::fs::read(&har).unwrap()).unwrap();
    assert!(
        log["log"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["request"]["url"] == server.url("/")),
        "{log}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn extension_drives_edge() {
    let Ok(found) = Chromium::executable_path(&chromiumoxide::LaunchOptions {
        browser_channel: Some("msedge".into()),
        ..Default::default()
    }) else {
        assert!(
            std::env::var_os("CHROMIUMOXIDE_REQUIRE_EDGE").is_none(),
            "no Edge installed"
        );
        return;
    };
    // The channel's executable, not the default Chromium's.
    assert!(
        found
            .executable
            .to_string_lossy()
            .to_lowercase()
            .contains("edge"),
        "{:?}",
        found.executable
    );
    let Some(extension) = crate::extension::built("edge") else {
        return;
    };
    let profile = std::env::temp_dir().join(format!("cox-ext-edge-{}", rand_suffix()));
    let browser = Chromium::launch_with_extension(ExtensionLaunchOptions {
        executable_path: Some(found.executable),
        extension_path: extension,
        user_data_dir: profile.clone(),
        args: if cfg!(target_os = "linux") {
            vec!["--no-sandbox".into()]
        } else {
            Vec::new()
        },
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(browser.version().await.unwrap().starts_with("Chrome/"));
    let page = browser.contexts()[0].new_page().await.unwrap();
    page.set_content("<button onclick=\"this.textContent='ok'\">go</button>")
        .await
        .unwrap();
    page.click("button").await.unwrap();
    assert_eq!(page.locator("button").inner_text().await.unwrap(), "ok");
    let ua: String = page.evaluate("navigator.userAgent").await.unwrap();
    assert!(ua.contains("Edg/"), "{ua}");
    browser.close().await.unwrap();
    let _ = std::fs::remove_dir_all(&profile);
}
