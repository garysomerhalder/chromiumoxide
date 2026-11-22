//! Playwright's `clientCertificates` option: TLS client certificates per
//! origin, for contexts and `APIRequestContext`.

use std::path::PathBuf;

/// One client certificate and the origin it is presented to. Give either
/// `cert` + `key` (PEM or DER; the key may be an encrypted PKCS#8 PEM with
/// `passphrase`), or `pfx` (PKCS#12, with `passphrase`). Each can be a path
/// or the bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientCertificate {
    /// `https://host[:port]`; the certificate is used for exactly this origin.
    pub origin: String,
    pub cert_path: Option<PathBuf>,
    pub cert: Option<Vec<u8>>,
    pub key_path: Option<PathBuf>,
    pub key: Option<Vec<u8>>,
    pub pfx_path: Option<PathBuf>,
    pub pfx: Option<Vec<u8>>,
    pub passphrase: Option<String>,
}

/// The certificate material once paths are read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientIdentity {
    pub cert: Option<Vec<u8>>,
    pub key: Option<Vec<u8>>,
    pub pfx: Option<Vec<u8>>,
    pub passphrase: Option<String>,
}

/// Playwright's `verifyClientCertificates`.
pub fn validate(list: &[ClientCertificate]) -> Result<(), String> {
    for c in list {
        if c.origin.is_empty() {
            return Err("clientCertificates.origin is required".into());
        }
        let cert = c.cert.is_some() || c.cert_path.is_some();
        let key = c.key.is_some() || c.key_path.is_some();
        let pfx = c.pfx.is_some() || c.pfx_path.is_some();
        if !cert && !key && c.passphrase.is_none() && !pfx {
            return Err("None of cert, key, passphrase or pfx is specified".into());
        }
        if cert && !key {
            return Err("cert is specified without key".into());
        }
        if !cert && key {
            return Err("key is specified without cert".into());
        }
        if pfx && (cert || key) {
            return Err("pfx is specified together with cert, key or passphrase".into());
        }
    }
    Ok(())
}

/// `new URL(origin).origin`: scheme, host and port (default ports dropped).
pub fn normalize_origin(origin: &str) -> Option<String> {
    let u = url::Url::parse(origin).ok()?;
    match u.origin() {
        url::Origin::Tuple(..) => Some(u.origin().ascii_serialization()),
        url::Origin::Opaque(_) => None,
    }
}

/// Whether `url` belongs to `origin` (both normalized).
pub fn url_has_origin(url: &str, origin: &str) -> bool {
    match (normalize_origin(url), normalize_origin(origin)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pem() -> ClientCertificate {
        ClientCertificate {
            origin: "https://a.test".into(),
            cert: Some(b"c".to_vec()),
            key: Some(b"k".to_vec()),
            ..Default::default()
        }
    }

    #[test]
    fn validation_follows_playwright() {
        assert_eq!(validate(&[pem()]), Ok(()));
        let pfx = ClientCertificate {
            origin: "https://a.test".into(),
            pfx_path: Some("/c.pfx".into()),
            passphrase: Some("p".into()),
            ..Default::default()
        };
        assert_eq!(validate(std::slice::from_ref(&pfx)), Ok(()));
        let cases = [
            (
                ClientCertificate {
                    origin: String::new(),
                    ..pem()
                },
                "clientCertificates.origin is required",
            ),
            (
                ClientCertificate {
                    origin: "https://a.test".into(),
                    ..Default::default()
                },
                "None of cert, key, passphrase or pfx is specified",
            ),
            (
                ClientCertificate { key: None, ..pem() },
                "cert is specified without key",
            ),
            (
                ClientCertificate {
                    cert: None,
                    ..pem()
                },
                "key is specified without cert",
            ),
            (
                ClientCertificate {
                    pfx: Some(vec![1]),
                    ..pem()
                },
                "pfx is specified together with cert, key or passphrase",
            ),
        ];
        for (c, msg) in cases {
            assert_eq!(validate(&[c]), Err(msg.to_owned()));
        }
    }

    #[test]
    fn origins_compare_scheme_host_and_port() {
        assert!(url_has_origin("https://a.test/x?y", "https://a.test"));
        assert!(url_has_origin("https://a.test:443/", "https://a.test"));
        assert!(url_has_origin(
            "https://a.test:8443/",
            "https://a.test:8443/"
        ));
        assert!(!url_has_origin("https://a.test:8443/", "https://a.test"));
        assert!(!url_has_origin("http://a.test/", "https://a.test"));
        assert!(!url_has_origin("https://b.test/", "https://a.test"));
    }
}
