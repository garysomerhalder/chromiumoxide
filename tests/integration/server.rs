//! A tiny HTTP server for integration tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// path -> (content type, body, delay ms)
type Routes = Arc<Mutex<HashMap<String, (String, Vec<u8>, u64)>>>;

/// path -> (status line, headers, body)
type Custom = Arc<Mutex<HashMap<String, (String, Vec<(String, String)>, Vec<u8>)>>>;

pub struct TestServer {
    pub port: u16,
    routes: Routes,
    custom: Custom,
}

/// Reads the request head and a `Content-Length` body.
async fn read_request(sock: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
    let mut data = Vec::new();
    let mut buf = vec![0u8; 16384];
    loop {
        let n = sock.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..end]).to_string();
            let len = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            while data.len() < end + 4 + len {
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
            }
            let body = data[end + 4..].to_vec();
            return (head, body);
        }
    }
    (String::from_utf8_lossy(&data).to_string(), Vec::new())
}

impl TestServer {
    pub async fn start() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Routes = Arc::default();
        let custom: Custom = Arc::default();
        let r = routes.clone();
        let c = custom.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let routes = r.clone();
                let custom = c.clone();
                tokio::spawn(async move {
                    let (req, req_body) = read_request(&mut sock).await;
                    let method = req.split_whitespace().next().unwrap_or("GET").to_owned();
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let found = custom.lock().unwrap().get(&path).cloned();
                    if let Some((status, headers, body)) = found {
                        let mut head = format!("HTTP/1.1 {status}\r\n");
                        for (k, v) in &headers {
                            head.push_str(&format!("{k}: {v}\r\n"));
                        }
                        head.push_str(&format!(
                            "Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        ));
                        let _ = sock.write_all(head.as_bytes()).await;
                        let _ = sock.write_all(&body).await;
                        return;
                    }
                    if path == "/headers" {
                        let header = |name: &str| {
                            req.lines()
                                .find_map(|l| {
                                    let (k, v) = l.split_once(':')?;
                                    k.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
                                })
                                .map_or("null".to_owned(), |v| format!("{v:?}"))
                        };
                        let body = format!(
                            "{{\"foo\":{},\"cookie\":{}}}",
                            header("x-foo"),
                            header("cookie")
                        );
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = sock.write_all(head.as_bytes()).await;
                        let _ = sock.write_all(body.as_bytes()).await;
                        return;
                    }
                    if path == "/echo" {
                        let body = format!(
                            "{{\"method\":{:?},\"body\":{:?}}}",
                            method,
                            String::from_utf8_lossy(&req_body)
                        );
                        let head = format!(
                            "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = sock.write_all(head.as_bytes()).await;
                        let _ = sock.write_all(body.as_bytes()).await;
                        return;
                    }
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
        TestServer {
            port,
            routes,
            custom,
        }
    }

    /// Serves `body` at `path`, optionally after `delay_ms`.
    pub fn route(&self, path: &str, content_type: &str, body: impl Into<Vec<u8>>, delay_ms: u64) {
        self.routes.lock().unwrap().insert(
            path.to_owned(),
            (content_type.to_owned(), body.into(), delay_ms),
        );
    }

    /// Serves a full response (status line such as `302 Found`, headers, body) at `path`.
    pub fn respond(
        &self,
        path: &str,
        status: &str,
        headers: &[(&str, &str)],
        body: impl Into<Vec<u8>>,
    ) {
        self.custom.lock().unwrap().insert(
            path.to_owned(),
            (
                status.to_owned(),
                headers
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
                body.into(),
            ),
        );
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}
