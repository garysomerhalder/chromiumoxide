//! Playwright's `_electron` against a real Electron (`ELECTRON_PATH`, or the
//! `electron` npm package found from the working directory), under Xvfb.
//! `CHROMIUMOXIDE_REQUIRE_ELECTRON=1` fails instead of skipping without one.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use chromiumoxide::{Electron, ElectronEvent, ElectronLaunchOptions};
use futures::StreamExt;

fn electron_executable() -> Option<PathBuf> {
    let found = std::env::var_os("ELECTRON_PATH")
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .or_else(chromiumoxide::Electron::find_executable);
    if found.is_none() {
        assert!(
            std::env::var_os("CHROMIUMOXIDE_REQUIRE_ELECTRON").is_none(),
            "Electron is required (set ELECTRON_PATH)"
        );
        eprintln!("no Electron; skipping");
    }
    found
}

/// An X display for Electron: `DISPLAY` if set, else a private Xvfb.
struct Display {
    name: String,
    xvfb: Option<Child>,
}

impl Drop for Display {
    fn drop(&mut self) {
        if let Some(x) = &mut self.xvfb {
            let _ = x.kill();
        }
    }
}

fn display() -> Option<Display> {
    if let Ok(name) = std::env::var("DISPLAY") {
        return Some(Display { name, xvfb: None });
    }
    let n = 90 + std::process::id() % 400;
    let name = format!(":{n}");
    let xvfb = Command::new("Xvfb")
        .args([
            name.as_str(),
            "-screen",
            "0",
            "1280x800x24",
            "-nolisten",
            "tcp",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let socket = format!("/tmp/.X11-unix/X{n}");
    for _ in 0..100 {
        if std::path::Path::new(&socket).exists() {
            return Some(Display {
                name,
                xvfb: Some(xvfb),
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

fn fixture() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/electron/main.js"
    )
    .to_owned()
}

#[tokio::test]
async fn electron_app_windows_main_process_and_close() {
    let Some(exe) = electron_executable() else {
        return;
    };
    let display = display().expect("an X display (Xvfb)");
    let app = Electron::launch(ElectronLaunchOptions {
        executable_path: Some(exe),
        args: vec![fixture()],
        env: Some(vec![("DISPLAY".into(), display.name.clone())]),
        ..Default::default()
    })
    .await
    .unwrap();
    let mut events = app.events();

    // The first window, as a page of the app's context.
    let window = app.first_window().await.unwrap();
    window
        .wait_for_load_state(chromiumoxide::WaitUntil::Load)
        .await
        .unwrap();
    assert_eq!(window.title().await.unwrap(), "Hello");
    assert_eq!(
        window
            .locator("h1")
            .text_content()
            .await
            .unwrap()
            .as_deref(),
        Some("Electron")
    );
    assert_eq!(app.windows().len(), 1);
    assert_eq!(app.context().pages().len(), 1);
    // No default viewport: the page is as big as the window.
    assert_eq!(window.evaluate::<i64>("innerWidth").await.unwrap(), 400);

    // `evaluate(fn, arg)` runs `fn(electron, arg)` in the main process.
    assert!(
        app.evaluate::<bool>("({ app }) => app.isReady()")
            .await
            .unwrap()
    );
    assert_eq!(
        app.evaluate_with::<i64, _>("(electron, n) => n * 2", 21)
            .await
            .unwrap(),
        42
    );
    // `browserWindow(page)`: the BrowserWindow of a page.
    let bw = app.browser_window(&window).await.unwrap();
    assert_eq!(
        bw.evaluate::<i64>("w => w.getBounds().width")
            .await
            .unwrap(),
        400
    );

    // Main-process console messages and new windows are events.
    app.evaluate::<()>("() => { console.log('hi from main', 7); }")
        .await
        .unwrap();
    app.evaluate::<()>(
        "({ BrowserWindow }) => { const w = new BrowserWindow({ width: 300, height: 200 }); \
         w.loadURL('data:text/html,<title>Second</title>'); }",
    )
    .await
    .unwrap();
    let mut console = None;
    let mut second = None;
    while console.is_none() || second.is_none() {
        match tokio::time::timeout(Duration::from_secs(20), events.next()).await {
            Ok(Some(ElectronEvent::Console(m))) => console = Some(m),
            Ok(Some(ElectronEvent::Window(p))) => second = Some(p),
            Ok(Some(_)) => {}
            other => panic!("no event: {:?}", other.map(|e| e.map(|_| ()))),
        }
    }
    let console = console.unwrap();
    assert_eq!(console.kind(), "log");
    assert_eq!(console.text(), "hi from main 7");
    let second = second.unwrap();
    second
        .wait_for_load_state(chromiumoxide::WaitUntil::Load)
        .await
        .unwrap();
    assert_eq!(second.title().await.unwrap(), "Second");
    assert_eq!(app.windows().len(), 2);

    // `process()` and `close()`: the app quits.
    let pid = app.process().pid();
    assert!(pid > 0);
    app.close().await.unwrap();
    assert!(
        std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |s| s.contains(") Z")),
        "Electron {pid} still running"
    );
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(e) = events.next().await {
            if matches!(e, ElectronEvent::Close) {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(closed.ok(), Some(true), "close event");
}
