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
