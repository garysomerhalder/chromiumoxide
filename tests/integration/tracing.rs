//! Tracing to a Playwright-style trace.zip (layout recorded from Playwright 1.63).

use std::time::Duration;

use chromiumoxide::{Chromium, ContextOptions, LaunchOptions, TracingStart};
use serde_json::Value;

use crate::server::TestServer;

/// Lists and reads the zip with Python's zipfile (which also checks CRCs).
fn unzip(path: &std::path::Path) -> Value {
    let script = "import sys, zipfile, json, base64\n\
                  z = zipfile.ZipFile(sys.argv[1])\n\
                  assert z.testzip() is None\n\
                  print(json.dumps({n: base64.b64encode(z.read(n)).decode() for n in z.namelist()}))";
    let out = std::process::Command::new("python3")
        .args(["-c", script])
        .arg(path)
        .output()
        .expect("python3");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn file(files: &Value, name: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(
            files[name]
                .as_str()
                .unwrap_or_else(|| panic!("missing {name}")),
        )
        .unwrap()
}

#[tokio::test]
async fn trace_zip_like_playwright() {
    let server = TestServer::start().await;
    server.route("/page", "text/html", "<p>traced</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    context
        .tracing()
        .start(TracingStart {
            screenshots: true,
            title: Some("my trace".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.set_content("<button onclick=\"console.log(1)\">B</button>")
        .await
        .unwrap();
    page.click("button").await.unwrap();
    page.goto(&server.url("/page")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let dir = std::env::temp_dir().join(format!("chromiumoxide-trace-{}", std::process::id()));
    let path = dir.join("trace.zip");
    context.tracing().stop(Some(&path)).await.unwrap();

    let files = unzip(&path);
    let names: Vec<String> = files.as_object().unwrap().keys().cloned().collect();
    for required in ["trace.trace", "trace.network", "trace.stacks"] {
        assert!(names.iter().any(|n| n == required), "{names:?}");
    }
    let events: Vec<Value> = String::from_utf8(file(&files, "trace.trace"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(events[0]["type"], "context-options");
    assert_eq!(events[0]["title"], "my trace");
    let action = |method: &str| {
        events
            .iter()
            .find(|e| e["type"] == "before" && e["method"] == method)
            .unwrap_or_else(|| panic!("no {method} action"))
            .clone()
    };
    let click = action("click");
    assert_eq!(click["class"], "Frame");
    assert_eq!(click["params"]["selector"], "button");
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "after" && e["callId"] == click["callId"])
    );
    action("setContent");
    assert_eq!(action("goto")["params"]["url"], server.url("/page"));
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "console" && e["text"] == "1")
    );
    let frames: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "screencast-frame")
        .collect();
    assert!(!frames.is_empty(), "no screencast frames");
    let jpeg = file(&files, frames[0]["file"].as_str().unwrap());
    assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    let network: Vec<Value> = String::from_utf8(file(&files, "trace.network"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(
        network.iter().any(|e| e["type"] == "resource-snapshot"
            && e["snapshot"]["request"]["url"] == server.url("/page")),
        "{network:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn trace_chunks_groups_and_har() {
    let server = TestServer::start().await;
    server.route("/one", "text/html", "<p>one</p>", 0);
    server.route("/two", "text/html", "<p>two</p>", 0);
    let dir = std::env::temp_dir().join(format!("chromiumoxide-chunks-{}", std::process::id()));
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    let tracing = context.tracing();
    tracing.start(TracingStart::default()).await.unwrap();
    tracing.group("first group").await.unwrap();
    page.goto(&server.url("/one")).await.unwrap();
    tracing.group_end().await.unwrap();
    let first = dir.join("one.zip");
    tracing.stop_chunk(Some(&first)).await.unwrap();
    tracing.start_chunk(Some("second")).await.unwrap();
    page.goto(&server.url("/two")).await.unwrap();
    let second = dir.join("two.zip");
    tracing.stop_chunk(Some(&second)).await.unwrap();
    tracing.stop(None).await.unwrap();

    let events = |path: &std::path::Path| -> Vec<Value> {
        String::from_utf8(file(&unzip(path), "trace.trace"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    };
    let one = events(&first);
    let group = one
        .iter()
        .find(|e| e["type"] == "before" && e["method"] == "tracingGroup")
        .expect("group");
    assert_eq!(group["params"]["name"], "first group");
    assert!(
        one.iter()
            .any(|e| e["type"] == "after" && e["callId"] == group["callId"])
    );
    let gotos = |ev: &[Value]| -> Vec<String> {
        ev.iter()
            .filter(|e| e["type"] == "before" && e["method"] == "goto")
            .map(|e| e["params"]["url"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(gotos(&one), [server.url("/one")]);
    let two = events(&second);
    assert_eq!(two[0]["type"], "context-options");
    assert_eq!(two[0]["title"], "second");
    assert_eq!(gotos(&two), [server.url("/two")]);

    // HAR on demand.
    let har_path = dir.join("on-demand.har");
    tracing.start_har(&har_path).await.unwrap();
    page.goto(&server.url("/one")).await.unwrap();
    tracing.stop_har().await.unwrap();
    let har: Value = serde_json::from_slice(&std::fs::read(&har_path).unwrap()).unwrap();
    assert!(
        har["log"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["request"]["url"] == server.url("/one"))
    );
    let _ = std::fs::remove_dir_all(&dir);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn trace_dom_snapshots_and_resources() {
    let server = TestServer::start().await;
    server.route(
        "/p.html",
        "text/html",
        "<link rel=stylesheet href=\"/s.css\"><button>B</button>",
        0,
    );
    server.route("/s.css", "text/css", "button { color: red; }", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    context
        .tracing()
        .start(TracingStart {
            snapshots: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&server.url("/p.html")).await.unwrap();
    page.click("button").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let dir = std::env::temp_dir().join(format!("cox-snap-trace-{}", std::process::id()));
    let path = dir.join("trace.zip");
    context.tracing().stop(Some(&path)).await.unwrap();

    let files = unzip(&path);
    // Layout recorded from Playwright 1.63 with snapshots on.
    let css = "resources/3fe33742618fac5890c933e5782ff6af7a2f181a.css";
    assert_eq!(file(&files, css), b"button { color: red; }");
    let network: Vec<Value> = String::from_utf8(file(&files, "trace.network"))
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let entry = network
        .iter()
        .find(|e| e["snapshot"]["request"]["url"] == server.url("/s.css"))
        .expect("stylesheet entry");
    assert_eq!(entry["snapshot"]["response"]["content"]["_file"], css);
    assert_eq!(
        entry["snapshot"]["response"]["content"]["mimeType"],
        "text/css"
    );

    let events: Vec<Value> = String::from_utf8(file(&files, "trace.trace"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let click = events
        .iter()
        .find(|e| e["type"] == "before" && e["method"] == "click")
        .unwrap();
    let snaps: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "frame-snapshot" && e["snapshot"]["callId"] == click["callId"])
        .collect();
    let phases: Vec<&str> = snaps
        .iter()
        .map(|e| e["snapshot"]["phase"].as_str().unwrap())
        .collect();
    assert_eq!(phases, ["before", "after"]);
    let after = &snaps[1]["snapshot"];
    assert_eq!(after["frameUrl"], server.url("/p.html"));
    assert_eq!(after["isMainFrame"], true);
    assert_eq!(
        after["viewport"],
        serde_json::json!({"width": 1280, "height": 720})
    );
    assert_eq!(
        after["html"][3],
        serde_json::json!(["BODY", {}, ["BUTTON", {}, "B"]])
    );
    assert!(after["frameId"].as_str().unwrap().starts_with("frame@"));
    let _ = std::fs::remove_dir_all(dir);
    browser.close().await.unwrap();
}
