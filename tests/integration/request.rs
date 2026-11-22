//! `APIRequestContext` (`context.request`, standalone contexts): behaviour
//! recorded from Playwright 1.63 against an equivalent server.

use std::collections::HashMap;

use chromiumoxide::{
    ApiRequestOptions, Chromium, ContextOptions, HttpCredentials, LaunchOptions, MultipartValue,
    NewCookie,
};
use serde_json::{Value, json};

use crate::server::TestServer;

#[tokio::test]
async fn context_request_shares_cookies_and_options() {
    let server = TestServer::start().await;
    server.respond("/r", "302 Found", &[("Location", "/dump")], "");
    server.respond(
        "/missing",
        "404 Not Found",
        &[("Content-Type", "text/plain")],
        "nope",
    );
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let base = server.url("");
    let ctx = browser
        .new_context(ContextOptions {
            base_url: Some(base.clone()),
            extra_http_headers: [("x-extra".to_owned(), "e".to_owned())].into(),
            ..Default::default()
        })
        .await
        .unwrap();
    ctx.add_cookies(&[NewCookie::new("k", "v").url(&base)])
        .await
        .unwrap();
    let page = ctx.new_page().await.unwrap();
    let ua: String = page.evaluate("navigator.userAgent").await.unwrap();
    let rq = ctx.request();

    let r = rq
        .get("/dump?x=1")
        .params(&[("y", "2"), ("z", "a b")])
        .await
        .unwrap();
    assert_eq!((r.status(), r.status_text(), r.ok()), (200, "OK", true));
    assert_eq!(r.url(), server.url("/dump?x=1&y=2&z=a+b"));
    let j: Value = r.json().await.unwrap();
    assert_eq!(j["method"], "GET");
    assert_eq!(j["path"], "/dump?x=1&y=2&z=a+b");
    assert_eq!(j["headers"]["user-agent"], json!(ua));
    assert_eq!(j["headers"]["accept"], "*/*");
    assert_eq!(j["headers"]["x-extra"], "e");
    assert_eq!(j["headers"]["cookie"], "k=v");
    let h: HashMap<String, String> = r.headers();
    assert_eq!(h["x-multi"], "a, b");
    assert_eq!(h["content-type"], "application/json");
    let arr = r.headers_array();
    assert!(arr.iter().any(|e| e.name == "X-Multi" && e.value == "a"));
    assert!(arr.iter().any(|e| e.name == "X-Multi" && e.value == "b"));
    // The response's cookie lands in the browser context.
    let cookies = ctx.cookies().await.unwrap();
    assert!(
        cookies
            .iter()
            .any(|c| c.name == "from_server" && c.value == "yes" && c.path == "/")
    );

    for (name, builder, ctype, body) in [
        (
            "json",
            rq.post("/dump").data(json!({"a": 1})),
            "application/json",
            "{\"a\":1}",
        ),
        (
            "string",
            rq.post("/dump").data_text("hello"),
            "application/octet-stream",
            "hello",
        ),
        (
            "buffer",
            rq.post("/dump").data_bytes(b"bin".to_vec()),
            "application/octet-stream",
            "bin",
        ),
        (
            "form",
            rq.post("/dump").form(&[("a", "1"), ("b", "x y")]),
            "application/x-www-form-urlencoded",
            "a=1&b=x+y",
        ),
    ] {
        let j: Value = builder.await.unwrap().json().await.unwrap();
        assert_eq!(j["headers"]["content-type"], ctype, "{name}");
        assert_eq!(j["body"], body, "{name}");
        assert_eq!(
            j["headers"]["content-length"],
            body.len().to_string(),
            "{name}"
        );
    }
    let j: Value = rq
        .post("/dump")
        .multipart(vec![
            ("f", MultipartValue::Text("v".into())),
            (
                "file",
                MultipartValue::File {
                    name: "a.txt".into(),
                    mime_type: "text/plain".into(),
                    buffer: b"hi".to_vec(),
                },
            ),
        ])
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ctype = j["headers"]["content-type"].as_str().unwrap();
    let boundary = ctype
        .strip_prefix("multipart/form-data; boundary=")
        .expect(ctype);
    assert!(boundary.starts_with("----WebKitFormBoundary"));
    assert_eq!(
        j["body"],
        format!(
            "--{boundary}\r\ncontent-disposition: form-data; name=\"f\"\r\n\r\nv\r\n--{boundary}\r\ncontent-disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\ncontent-type: text/plain\r\n\r\nhi\r\n--{boundary}--\r\n"
        )
    );

    // Redirects: POST becomes GET without a body; max_redirects(0) stops.
    let r = rq.post("/r").data_text("x").await.unwrap();
    assert_eq!(r.url(), server.url("/dump"));
    let j: Value = r.json().await.unwrap();
    assert_eq!(
        (j["method"].as_str(), j["body"].as_str()),
        (Some("GET"), Some(""))
    );
    let r = rq.get("/r").max_redirects(0).await.unwrap();
    assert_eq!(r.status(), 302);
    assert_eq!(r.headers()["location"], "/dump");

    let err = rq
        .get("/missing")
        .fail_on_status_code(true)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "404 Not Found\nResponse text:\nnope");
    let r = rq.get("/missing").await.unwrap();
    assert_eq!((r.status(), r.status_text()), (404, "Not Found"));
    assert_eq!(r.text().await.unwrap(), "nope");

    let j: Value = rq
        .fetch("/dump")
        .method("PATCH")
        .header("X-Own", "o")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        (j["method"].as_str(), j["headers"]["x-own"].as_str()),
        (Some("PATCH"), Some("o"))
    );
    for (m, b) in [("PUT", rq.put("/dump")), ("DELETE", rq.delete("/dump"))] {
        let j: Value = b.await.unwrap().json().await.unwrap();
        assert_eq!(j["method"], m);
    }
    let r = rq.head("/dump").await.unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.body().await.unwrap().is_empty());
    r.dispose().await;
    assert_eq!(
        r.body().await.unwrap_err().to_string(),
        "Response has been disposed"
    );

    // page.request is the context's.
    let j: Value = page
        .request()
        .get("/dump")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j["headers"]["x-extra"], "e");

    // HTTP credentials answer a Basic challenge.
    let ctx2 = browser
        .new_context(ContextOptions {
            base_url: Some(base.clone()),
            http_credentials: Some(HttpCredentials {
                username: "user".into(),
                password: "pass".into(),
                origin: None,
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let r = ctx2.request().get("/protected").await.unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.text().await.unwrap().contains("secret ok"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn standalone_request_context() {
    let server = TestServer::start().await;
    let rq = Chromium::request_context(ApiRequestOptions {
        base_url: Some(server.url("")),
        ..Default::default()
    })
    .await
    .unwrap();
    let j: Value = rq.get("/dump").await.unwrap().json().await.unwrap();
    assert!(
        j["headers"]["user-agent"]
            .as_str()
            .unwrap()
            .starts_with("chromiumoxide/"),
        "{j}"
    );
    assert!(j["headers"].get("cookie").is_none());
    // Its own cookie jar.
    let j: Value = rq.get("/dump").await.unwrap().json().await.unwrap();
    assert_eq!(j["headers"]["cookie"], "from_server=yes");
    let state = rq.storage_state().await.unwrap();
    let names: Vec<_> = state
        .cookies
        .iter()
        .map(|c| (c.name.as_str(), c.domain.as_str(), c.path.as_str()))
        .collect();
    assert_eq!(names, [("from_server", "127.0.0.1", "/")]);
    rq.dispose().await;
    assert_eq!(
        rq.get("/dump").await.unwrap_err().to_string(),
        "Target page, context or browser has been closed"
    );
}
