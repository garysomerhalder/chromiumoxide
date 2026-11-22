//! Trace files (Playwright's `trace.zip` layout) and the ZIP container.

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
    fn context_options_event() {
        let e = context_options_event("chromium", 1280, 720, 1_700_000_000_000.0, 12.5, "t");
        let v: serde_json::Value = serde_json::from_str(&e).unwrap();
        assert_eq!(v["type"], "context-options");
        assert_eq!(v["version"], TRACE_VERSION);
        assert_eq!(v["browserName"], "chromium");
        assert_eq!(v["options"]["viewport"]["width"], 1280);
        assert_eq!(v["title"], "t");
    }
}
