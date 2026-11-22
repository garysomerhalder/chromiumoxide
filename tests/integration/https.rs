//! `ignoreHTTPSErrors` against a self-signed HTTPS server (`openssl s_server`).

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use chromiumoxide::{Chromium, ContextOptions, LaunchOptions};

struct Server(Child, std::path::PathBuf);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn https_server() -> Option<(Server, u16)> {
    let dir = std::env::temp_dir().join(format!("cox-https-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let ok = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
        ])
        .arg("-keyout")
        .arg(dir.join("k.pem"))
        .arg("-out")
        .arg(dir.join("c.pem"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?
        .success();
    if !ok {
        return None;
    }
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()?
        .port();
    let child = Command::new("openssl")
        .args(["s_server", "-www", "-quiet", "-accept"])
        .arg(port.to_string())
        .arg("-cert")
        .arg(dir.join("c.pem"))
        .arg("-key")
        .arg(dir.join("k.pem"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let server = Server(child, dir);
    for _ in 0..50 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Some((server, port));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[tokio::test]
async fn ignore_https_errors() {
    let Some((_server, port)) = https_server() else {
        eprintln!("no openssl; skipping");
        return;
    };
    let url = format!("https://localhost:{port}/");
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();

    let strict = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let page = strict.new_page().await.unwrap();
    let err = page.goto(&url).await.unwrap_err();
    assert!(
        err.to_string().contains("net::ERR_CERT_AUTHORITY_INVALID"),
        "{err}"
    );

    let lax = browser
        .new_context(ContextOptions {
            ignore_https_errors: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let page = lax.new_page().await.unwrap();
    let response = page.goto(&url).await.unwrap().expect("a response");
    assert_eq!(response.status, 200);
    assert!(page.content().await.unwrap().contains("s_server"));
    browser.close().await.unwrap();
}
