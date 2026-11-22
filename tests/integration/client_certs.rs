//! `clientCertificates` (TLS client authentication) against an
//! `openssl s_server` that requires a certificate signed by a test CA. With
//! `-www` it answers with a page naming the client certificate's subject.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use chromiumoxide::{
    ApiRequestOptions, Chromium, ClientCertificate, ContextOptions, LaunchOptions,
};

struct MtlsServer {
    child: Child,
    dir: PathBuf,
    port: u16,
}

impl Drop for MtlsServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
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
fn mtls_server() -> Option<MtlsServer> {
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
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()?
        .port();
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
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Some(MtlsServer { child, dir, port });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
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
