//! `clientCertificates` (TLS client authentication) against an
//! `openssl s_server` that requires a certificate signed by a test CA. With
//! `-www` it answers with a page naming the client certificate's subject.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use chromiumoxide::{
    ApiRequestOptions, Chromium, ClientCertificate, ContextOptions, LaunchOptions,
};

pub(crate) struct MtlsServer {
    children: Vec<Child>,
    pub(crate) dir: PathBuf,
    pub(crate) port: u16,
}

impl Drop for MtlsServer {
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn openssl(dir: &Path, args: &[&str]) -> bool {
    Command::new("openssl")
        .current_dir(dir)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A CA, a server certificate for localhost, a client certificate (CN=Alice)
/// as PEM, as an encrypted PEM key and as a PFX (passphrase `secret`).
pub(crate) fn mtls_server() -> Option<MtlsServer> {
    let dir = std::env::temp_dir().join(format!("cox-mtls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(
        dir.join("ext.cnf"),
        "subjectAltName=DNS:localhost,IP:127.0.0.1\n",
    )
    .ok()?;
    let steps: &[&[&str]] = &[
        &[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=TestCA",
            "-keyout",
            "ca.key",
            "-out",
            "ca.pem",
        ],
        &[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            "/CN=localhost",
            "-keyout",
            "server.key",
            "-out",
            "server.csr",
        ],
        &[
            "x509",
            "-req",
            "-in",
            "server.csr",
            "-CA",
            "ca.pem",
            "-CAkey",
            "ca.key",
            "-CAcreateserial",
            "-days",
            "1",
            "-extfile",
            "ext.cnf",
            "-out",
            "server.pem",
        ],
        &[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            "/CN=Alice",
            "-keyout",
            "client.key",
            "-out",
            "client.csr",
        ],
        &[
            "x509",
            "-req",
            "-in",
            "client.csr",
            "-CA",
            "ca.pem",
            "-CAkey",
            "ca.key",
            "-CAcreateserial",
            "-days",
            "1",
            "-out",
            "client.pem",
        ],
        &[
            "pkcs12",
            "-export",
            "-inkey",
            "client.key",
            "-in",
            "client.pem",
            "-out",
            "client.pfx",
            "-passout",
            "pass:secret",
        ],
        &[
            "pkcs8",
            "-topk8",
            "-in",
            "client.key",
            "-out",
            "client.enc.key",
            "-passout",
            "pass:secret",
        ],
    ];
    for step in steps {
        if !openssl(&dir, step) {
            eprintln!("openssl {step:?} failed; skipping");
            return None;
        }
    }
    // `s_server` serves one connection at a time, and a browser may hold one
    // open (WebKit preconnects without a certificate). Like a real server,
    // the fixture takes several at once: a front port hands each connection
    // to an idle `s_server` of a small pool.
    let mut children = Vec::new();
    let mut backends = Vec::new();
    for _ in 0..4 {
        let port = free_port()?;
        let child = Command::new("openssl")
            .current_dir(&dir)
            .args([
                "s_server",
                "-accept",
                &port.to_string(),
                "-cert",
                "server.pem",
                "-key",
                "server.key",
            ])
            .args(["-CAfile", "ca.pem", "-Verify", "1", "-www", "-quiet"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        children.push(child);
        backends.push(port);
    }
    let ready = backends.iter().all(|&port| {
        (0..100).any(|_| {
            let up = std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();
            if !up {
                std::thread::sleep(Duration::from_millis(20));
            }
            up
        })
    });
    if !ready {
        for mut c in children {
            let _ = c.kill();
        }
        return None;
    }
    let front = crate::server::browsable_std_listener();
    let port = front.local_addr().ok()?.port();
    let idle = Arc::new((Mutex::new(backends), Condvar::new()));
    std::thread::spawn(move || {
        for conn in front.incoming().flatten() {
            let idle = idle.clone();
            std::thread::spawn(move || {
                let backend = {
                    let (lock, cv) = &*idle;
                    let mut free = lock.lock().unwrap();
                    while free.is_empty() {
                        free = cv.wait(free).unwrap();
                    }
                    free.pop().unwrap()
                };
                if let Ok(up) = std::net::TcpStream::connect(("127.0.0.1", backend)) {
                    pipe(conn, up);
                }
                let (lock, cv) = &*idle;
                lock.lock().unwrap().push(backend);
                cv.notify_one();
            });
        }
    });
    Some(MtlsServer {
        children,
        dir,
        port,
    })
}

fn free_port() -> Option<u16> {
    Some(
        std::net::TcpListener::bind("127.0.0.1:0")
            .ok()?
            .local_addr()
            .ok()?
            .port(),
    )
}

/// Copies both ways until either side closes.
fn pipe(a: std::net::TcpStream, b: std::net::TcpStream) {
    let (Ok(a2), Ok(b2)) = (a.try_clone(), b.try_clone()) else {
        return;
    };
    let t = std::thread::spawn(move || {
        let (mut r, mut w) = (a2, b2);
        let _ = std::io::copy(&mut r, &mut w);
        let _ = w.shutdown(std::net::Shutdown::Both);
        let _ = r.shutdown(std::net::Shutdown::Both);
    });
    let (mut r, mut w) = (b, a);
    let _ = std::io::copy(&mut r, &mut w);
    let _ = w.shutdown(std::net::Shutdown::Both);
    let _ = r.shutdown(std::net::Shutdown::Both);
    let _ = t.join();
}

#[tokio::test]
async fn client_certificates_authenticate_to_their_origin() {
    let Some(server) = mtls_server() else {
        return;
    };
    let origin = format!("https://localhost:{}", server.port);
    let dir = &server.dir;
    let pem = ClientCertificate {
        origin: origin.clone(),
        cert_path: Some(dir.join("client.pem")),
        key_path: Some(dir.join("client.key")),
        ..Default::default()
    };

    // request.newContext({ clientCertificates }) with PEM files.
    let request = Chromium::request_context(ApiRequestOptions {
        ignore_https_errors: true,
        client_certificates: vec![pem.clone()],
        ..Default::default()
    })
    .await
    .unwrap();
    let body = request
        .get(&format!("{origin}/"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("Subject: CN=Alice"), "{body}");
    // Without a certificate the server refuses the handshake.
    let bare = Chromium::request_context(ApiRequestOptions {
        ignore_https_errors: true,
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(bare.get(&format!("{origin}/")).await.is_err());

    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    // A context with a PFX and its passphrase (bytes, not a path).
    let context = browser
        .new_context(ContextOptions {
            ignore_https_errors: true,
            client_certificates: vec![ClientCertificate {
                origin: origin.clone(),
                pfx: Some(std::fs::read(dir.join("client.pfx")).unwrap()),
                passphrase: Some("secret".into()),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&format!("{origin}/")).await.unwrap();
    let text = page
        .locator("body")
        .text_content()
        .await
        .unwrap()
        .unwrap_or_default();
    assert!(text.contains("Subject: CN=Alice"), "{text}");
    // context.request shares the certificates.
    let body = context
        .request()
        .get(&format!("{origin}/"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("Subject: CN=Alice"), "{body}");
    context.close().await.unwrap();

    // An encrypted PEM key with its passphrase.
    let context = browser
        .new_context(ContextOptions {
            ignore_https_errors: true,
            client_certificates: vec![ClientCertificate {
                key_path: Some(dir.join("client.enc.key")),
                passphrase: Some("secret".into()),
                ..pem.clone()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    page.goto(&format!("{origin}/")).await.unwrap();
    let text = page
        .locator("body")
        .text_content()
        .await
        .unwrap()
        .unwrap_or_default();
    assert!(text.contains("Subject: CN=Alice"), "{text}");
    context.close().await.unwrap();

    // Certificates only apply to their own origin (scheme, host and port).
    let context = browser
        .new_context(ContextOptions {
            ignore_https_errors: true,
            client_certificates: vec![ClientCertificate {
                origin: format!("https://127.0.0.1:{}", server.port),
                ..pem.clone()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    assert!(
        page.goto(&format!("{origin}/")).await.is_err(),
        "no certificate for localhost"
    );
    context.close().await.unwrap();

    // Playwright's validation.
    let err = browser
        .new_context(ContextOptions {
            client_certificates: vec![ClientCertificate {
                key_path: None,
                ..pem.clone()
            }],
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("cert is specified without key"),
        "{err}"
    );
    let err = browser
        .new_context(ContextOptions {
            client_certificates: vec![ClientCertificate {
                origin: String::new(),
                ..pem.clone()
            }],
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("clientCertificates.origin is required"),
        "{err}"
    );
    let err = browser
        .new_context(ContextOptions {
            client_certificates: vec![ClientCertificate {
                pfx: Some(vec![1, 2, 3]),
                ..pem.clone()
            }],
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("pfx is specified together with cert, key or passphrase"),
        "{err}"
    );
    browser.close().await.unwrap();
}
