//! A small HTTP/1.1 client over tokio (TLS by rustls): one request per
//! connection, response header names and reason phrases kept as sent, as
//! Playwright's Node client reports them.

use std::io;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};

use crate::domain::http::{HttpRequest, HttpResponse};
use crate::ports::HttpClient;

/// The production HTTP client.
#[derive(Debug, Default, Clone, Copy)]
pub struct TcpHttpClient;

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn verified_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = RootCertStore::empty();
            for cert in rustls_native_certs::load_native_certs().certs {
                let _ = roots.add(cert);
            }
            Arc::new(
                ClientConfig::builder_with_provider(provider())
                    .with_safe_default_protocol_versions()
                    .expect("default TLS versions")
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

/// `ignoreHTTPSErrors`: any certificate is accepted.
#[derive(Debug)]
struct AcceptAll(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AcceptAll {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn insecure_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            Arc::new(
                ClientConfig::builder_with_provider(provider())
                    .with_safe_default_protocol_versions()
                    .expect("default TLS versions")
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(AcceptAll(provider())))
                    .with_no_client_auth(),
            )
        })
        .clone()
}

/// Milestones of a connection, in ms since the request started (-1: none).
struct Marks {
    lookup: (f64, f64),
    connect_start: f64,
    secure_start: f64,
    connect_end: f64,
}

struct Connection {
    stream: Box<dyn Stream>,
    peer: Option<std::net::SocketAddr>,
    tls: Option<(String, Option<Vec<u8>>)>,
    marks: Marks,
}

async fn connect(
    url: &url::Url,
    insecure: bool,
    started: std::time::Instant,
) -> io::Result<Connection> {
    let ms = || started.elapsed().as_secs_f64() * 1000.0;
    let host = url
        .host_str()
        .ok_or_else(|| invalid(format!("Invalid URL: {url}")))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| invalid(format!("Unsupported protocol: {}", url.scheme())))?;
    let mut marks = Marks {
        lookup: (-1.0, -1.0),
        connect_start: 0.0,
        secure_start: -1.0,
        connect_end: 0.0,
    };
    let addrs: Vec<std::net::SocketAddr> = match host.parse::<std::net::IpAddr>() {
        Ok(ip) => vec![(ip, port).into()],
        Err(_) => {
            // The lookup is the first step, so it starts at 0.
            let found = tokio::net::lookup_host((host.as_str(), port))
                .await?
                .collect();
            marks.lookup = (0.0, ms());
            marks.connect_start = ms();
            found
        }
    };
    let tcp = TcpStream::connect(&addrs[..]).await?;
    let peer = tcp.peer_addr().ok();
    match url.scheme() {
        "http" => {
            marks.connect_end = ms();
            Ok(Connection {
                stream: Box::new(tcp),
                peer,
                tls: None,
                marks,
            })
        }
        "https" => {
            marks.secure_start = ms();
            let config = if insecure {
                insecure_config()
            } else {
                verified_config()
            };
            let name = rustls::pki_types::ServerName::try_from(host)
                .map_err(|e| invalid(e.to_string()))?;
            let tls = TlsConnector::from(config).connect(name, tcp).await?;
            marks.connect_end = ms();
            let session = tls.get_ref().1;
            let protocol = match session.protocol_version() {
                Some(rustls::ProtocolVersion::TLSv1_3) => "TLSv1.3",
                Some(rustls::ProtocolVersion::TLSv1_2) => "TLSv1.2",
                _ => "unknown",
            }
            .to_owned();
            let cert = session
                .peer_certificates()
                .and_then(|c| c.first())
                .map(|c| c.as_ref().to_vec());
            Ok(Connection {
                stream: Box::new(tls),
                peer,
                tls: Some((protocol, cert)),
                marks,
            })
        }
        other => Err(invalid(format!("Unsupported protocol: {other}:"))),
    }
}

/// Decodes a `Transfer-Encoding: chunked` body.
fn dechunk(mut data: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = data
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| invalid("truncated chunk"))?;
        let size_line = String::from_utf8_lossy(&data[..end]);
        let size = usize::from_str_radix(size_line.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| invalid("bad chunk size"))?;
        data = &data[end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if data.len() < size {
            return Err(invalid("truncated chunk"));
        }
        out.extend_from_slice(&data[..size]);
        data = data.get(size + 2..).unwrap_or_default();
    }
}

async fn exchange(req: HttpRequest) -> io::Result<HttpResponse> {
    let url = url::Url::parse(&req.url).map_err(|e| invalid(format!("Invalid URL: {e}")))?;
    let started = std::time::Instant::now();
    let start_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or_default();
    let ms = || started.elapsed().as_secs_f64() * 1000.0;
    let conn = connect(&url, req.ignore_https_errors, started).await?;
    let mut stream = conn.stream;
    let mut target = url.path().to_owned();
    if let Some(q) = url.query() {
        target.push('?');
        target.push_str(q);
    }
    let host = match url.port() {
        Some(p) => format!("{}:{p}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_owned(),
    };
    let mut head = format!("{} {target} HTTP/1.1\r\nHost: {host}\r\n", req.method);
    for (k, v) in &req.headers {
        if k.eq_ignore_ascii_case("host") || k.eq_ignore_ascii_case("connection") {
            continue;
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    let request_start = ms();
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&req.body).await?;
    stream.flush().await?;
    let mut raw = Vec::new();
    let mut first = [0u8; 1];
    let n = stream.read(&mut first).await?;
    let response_start = ms();
    raw.extend_from_slice(&first[..n]);
    stream.read_to_end(&mut raw).await.or_else(|e| {
        // Servers that close TLS without notify still sent the whole response.
        if e.kind() == io::ErrorKind::UnexpectedEof && !raw.is_empty() {
            Ok(0)
        } else {
            Err(e)
        }
    })?;
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| invalid("malformed HTTP response"))?;
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let mut parts = status_line.splitn(3, ' ');
    let _version = parts.next();
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| invalid("malformed status line"))?;
    let status_text = parts.next().unwrap_or_default().to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    };
    let rest = &raw[end + 4..];
    let body = if req.method == "HEAD" || status == 204 || status == 304 {
        Vec::new()
    } else if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        dechunk(rest)?
    } else if let Some(len) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        rest[..len.min(rest.len())].to_vec()
    } else {
        rest.to_vec()
    };
    let m = conn.marks;
    Ok(HttpResponse {
        url: req.url,
        status,
        status_text,
        headers,
        body,
        server_addr: conn.peer.map(|p| (p.ip().to_string(), p.port())),
        tls: conn.tls,
        timing: Some(crate::domain::network::Timing {
            start_time,
            domain_lookup_start: m.lookup.0,
            domain_lookup_end: m.lookup.1,
            connect_start: m.connect_start,
            secure_connection_start: m.secure_start,
            connect_end: m.connect_end,
            request_start,
            response_start,
            response_end: ms(),
        }),
    })
}

impl HttpClient for TcpHttpClient {
    fn send(&self, request: HttpRequest) -> BoxFuture<'static, io::Result<HttpResponse>> {
        async move {
            match request.timeout_ms.filter(|t| *t > 0) {
                Some(ms) => tokio::time::timeout(Duration::from_millis(ms), exchange(request))
                    .await
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::TimedOut, format!("Timeout {ms}ms exceeded."))
                    })?,
                None => exchange(request).await,
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dechunks() {
        assert_eq!(
            dechunk(b"3\r\nabc\r\n2;x=1\r\nde\r\n0\r\n\r\n").unwrap(),
            b"abcde"
        );
        assert!(dechunk(b"5\r\nab").is_err());
    }
}
