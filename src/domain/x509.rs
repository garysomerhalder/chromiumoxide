//! Just enough X.509 (DER) for `securityDetails()`: subject and issuer common
//! names and the validity period.

/// What `securityDetails()` shows of a certificate.
#[derive(Debug, Clone, PartialEq)]
pub struct CertificateSummary {
    pub subject_name: Option<String>,
    pub issuer: Option<String>,
    /// Unix seconds.
    pub valid_from: f64,
    pub valid_to: f64,
}

/// Reads a DER certificate.
pub fn certificate_summary(der: &[u8]) -> Option<CertificateSummary> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// openssl req -x509 -subj "/C=US/O=Test Org/CN=issuer.example" -days 10
    const CERT: &str = "MIIBxzCCAW2gAwIBAgIUfRyoOdPNCHWNBSIAvjBx4izGoGYwCgYIKoZIzj0EAwIwOTELMAkGA1UEBhMCVVMxETAPBgNVBAoMCFRlc3QgT3JnMRcwFQYDVQQDDA5pc3N1ZXIuZXhhbXBsZTAeFw0yNjExMjIxNjQ4MjBaFw0yNjEyMDIxNjQ4MjBaMDkxCzAJBgNVBAYTAlVTMREwDwYDVQQKDAhUZXN0IE9yZzEXMBUGA1UEAwwOaXNzdWVyLmV4YW1wbGUwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAASaO/T/hTRjytJ4m8xEBzYklwSe21etSHZiMzxFDjSVcMGIBxOGPRitDWdDABTN3+wLcYMnyAX8KuLZch7+0s8Vo1MwUTAdBgNVHQ4EFgQUq4dhBBtW6XMDJeJ7Q/YYRTfpsIUwHwYDVR0jBBgwFoAUq4dhBBtW6XMDJeJ7Q/YYRTfpsIUwDwYDVR0TAQH/BAUwAwEB/zAKBggqhkjOPQQDAgNIADBFAiAvaNKk1G/lcj/0XvnlG2Ccq0lB16DTGWGVxx0QCwUgwwIhAPQob9Tw8bsSdh96So/c8wOTV7igLPyd0UPNTWNs8Y65";

    #[test]
    fn reads_names_and_validity() {
        let der = base64::engine::general_purpose::STANDARD
            .decode(CERT)
            .unwrap();
        assert_eq!(
            certificate_summary(&der).unwrap(),
            CertificateSummary {
                subject_name: Some("issuer.example".into()),
                issuer: Some("issuer.example".into()),
                valid_from: 1_795_366_100.0,
                valid_to: 1_796_230_100.0,
            }
        );
        assert!(certificate_summary(b"not der").is_none());
    }
}
