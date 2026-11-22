//! A tiny HTTP server for integration tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// path -> (content type, body, delay ms)
type Routes = Arc<Mutex<HashMap<String, (String, Vec<u8>, u64)>>>;

pub struct TestServer {
    pub port: u16,
    routes: Routes,
}

impl TestServer {
    pub async fn start() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Routes = Arc::default();
        let r = routes.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let routes = r.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16384];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let auth = req
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("Authorization: Basic ")
                                .or_else(|| l.strip_prefix("authorization: Basic "))
                        })
                        .map(|s| s.trim().to_owned());
                    let (status, ctype, body, delay) = if path == "/protected" {
                        let expected =
                            base64::engine::general_purpose::STANDARD.encode("user:pass");
                        if auth.as_deref() == Some(expected.as_str()) {
                            (
                                "200 OK",
                                "text/html".to_owned(),
                                b"<body>secret ok</body>".to_vec(),
                                0,
                            )
                        } else {
                            (
                                "401 Unauthorized",
                                "text/html".to_owned(),
                                b"<body>denied</body>".to_vec(),
                                0,
                            )
                        }
                    } else if let Some((ct, body, delay)) =
                        routes.lock().unwrap().get(&path).cloned()
                    {
                        ("200 OK", ct, body, delay)
                    } else {
                        (
                            "404 Not Found",
                            "text/plain".to_owned(),
                            b"not found".to_vec(),
                            0,
                        )
                    };
                    if delay > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    }
                    let extra = if status.starts_with("401") {
                        "WWW-Authenticate: Basic realm=\"test\"\r\n"
                    } else {
                        ""
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&body).await;
                });
            }
        });
        TestServer { port, routes }
    }

    /// Serves `body` at `path`, optionally after `delay_ms`.
    pub fn route(&self, path: &str, content_type: &str, body: impl Into<Vec<u8>>, delay_ms: u64) {
        self.routes.lock().unwrap().insert(
            path.to_owned(),
            (content_type.to_owned(), body.into(), delay_ms),
        );
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}
