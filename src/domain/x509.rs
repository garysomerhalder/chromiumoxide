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
    let (tag, cert, _) = tlv(der)?;
    if tag != 0x30 {
        return None;
    }
    let (_, tbs, _) = tlv(cert)?;
    let mut fields = Vec::new();
    let mut rest = tbs;
    while !rest.is_empty() {
        let (tag, value, next) = tlv(rest)?;
        fields.push((tag, value));
        rest = next;
    }
    // An explicit version ([0]) shifts the fields by one.
    let base = usize::from(fields.first()?.0 == 0xA0);
    let issuer = fields.get(base + 2)?.1;
    let validity = fields.get(base + 3)?.1;
    let subject = fields.get(base + 4)?.1;
    let (t1, from, after) = tlv(validity)?;
    let (t2, to, _) = tlv(after)?;
    Some(CertificateSummary {
        subject_name: common_name(subject),
        issuer: common_name(issuer),
        valid_from: time(t1, from)?,
        valid_to: time(t2, to)?,
    })
}

/// One DER element: (tag, contents, the bytes after it).
fn tlv(data: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *data.first()?;
    let first = *data.get(1)? as usize;
    let (len, header) = if first < 0x80 {
        (first, 2)
    } else {
        let n = first & 0x7f;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *data.get(2 + i)? as usize;
        }
        (len, 2 + n)
    };
    let end = header.checked_add(len)?;
    Some((tag, data.get(header..end)?, data.get(end..)?))
}

const CN: &[u8] = &[0x55, 0x04, 0x03];

/// The last common name of a `Name`.
fn common_name(name: &[u8]) -> Option<String> {
    let mut found = None;
    let mut rest = name;
    while !rest.is_empty() {
        let (_, set, next) = tlv(rest)?;
        rest = next;
        let mut attrs = set;
        while !attrs.is_empty() {
            let (_, attr, after) = tlv(attrs)?;
            attrs = after;
            let (oid_tag, oid, value) = tlv(attr)?;
            if oid_tag == 0x06 && oid == CN {
                let (vtag, v, _) = tlv(value)?;
                found = Some(match vtag {
                    0x1E => String::from_utf16_lossy(
                        &v.chunks(2)
                            .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]))
                            .collect::<Vec<_>>(),
                    ),
                    _ => String::from_utf8_lossy(v).into_owned(),
                });
            }
        }
    }
    found
}

/// UTCTime / GeneralizedTime (UTC) in Unix seconds.
fn time(tag: u8, v: &[u8]) -> Option<f64> {
    let s = std::str::from_utf8(v).ok()?.trim_end_matches('Z');
    let num = |a: usize, b: usize| -> Option<i64> { s.get(a..b)?.parse().ok() };
    let (year, rest) = match tag {
        0x17 => {
            let yy = num(0, 2)?;
            (if yy < 50 { 2000 + yy } else { 1900 + yy }, 2)
        }
        0x18 => (num(0, 4)?, 4),
        _ => return None,
    };
    let (mo, d) = (num(rest, rest + 2)?, num(rest + 2, rest + 4)?);
    let (h, mi, sec) = (
        num(rest + 4, rest + 6)?,
        num(rest + 6, rest + 8)?,
        num(rest + 8, rest + 10).unwrap_or(0),
    );
    let (yy, m) = if mo <= 2 {
        (year - 1, mo + 9)
    } else {
        (year, mo - 3)
    };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some((days * 86_400 + h * 3600 + mi * 60 + sec) as f64)
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
