//! Trace files (Playwright's `trace.zip` layout) and the ZIP container.

use serde_json::json;

/// The trace format version written (Playwright 1.63 writes 9).
pub const TRACE_VERSION: u32 = 9;

/// CRC-32 (IEEE), as ZIP uses.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// A ZIP archive with the files stored uncompressed.
pub fn zip_stored(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let name = name.as_bytes();
        // Local file header.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // flags: UTF-8 names
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // time
        out.extend_from_slice(&0x21u16.to_le_bytes()); // date: 1980-01-01
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        // Central directory entry.
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0x21u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// The first event of a trace chunk.
pub fn context_options_event(
    browser: &str,
    width: u32,
    height: u32,
    wall_time: f64,
    monotonic_time: f64,
    title: &str,
    platform: &str,
) -> String {
    json!({
        "version": TRACE_VERSION,
        "type": "context-options",
        "origin": "library",
        "browserName": browser,
        "playwrightVersion": "1.63.0",
        "options": {"noDefaultViewport": false, "viewport": {"width": width, "height": height}},
        "platform": platform,
        "wallTime": wall_time,
        "monotonicTime": monotonic_time,
        "sdkLanguage": "javascript",
        "testIdAttributeName": "data-testid",
        "title": title,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_zlib() {
        assert_eq!(crc32(b"hello"), 907_060_870);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn stored_zip_layout() {
        let zip = zip_stored(&[("a.txt", b"hello".to_vec()), ("dir/b.bin", vec![0, 1, 2])]);
        // Local file header signature, then the central directory and its end.
        assert_eq!(&zip[..4], &[0x50, 0x4b, 0x03, 0x04]);
        let end = zip.len() - 22;
        assert_eq!(&zip[end..end + 4], &[0x50, 0x4b, 0x05, 0x06]);
        // Two entries.
        assert_eq!(u16::from_le_bytes([zip[end + 10], zip[end + 11]]), 2);
        assert!(zip.windows(5).any(|w| w == b"hello"));
    }

    #[test]
    fn context_options_event_shape() {
        let e = context_options_event(
            "chromium",
            1280,
            720,
            1_700_000_000_000.0,
            12.5,
            "t",
            "linux",
        );
        let v: serde_json::Value = serde_json::from_str(&e).unwrap();
        assert_eq!(v["type"], "context-options");
        assert_eq!(v["version"], TRACE_VERSION);
        assert_eq!(v["browserName"], "chromium");
        assert_eq!(v["options"]["viewport"]["width"], 1280);
        assert_eq!(v["title"], "t");
    }
}
