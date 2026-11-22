//! Trace files (Playwright's `trace.zip` layout) and the ZIP container.

use serde_json::json;

/// The trace format version written (Playwright 1.65 writes 10: no `stepId`,
/// which chromiumoxide never wrote).
pub const TRACE_VERSION: u32 = 10;

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

/// SHA-1 as lower-case hex (trace resource names).
pub fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
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
        "playwrightVersion": "1.65.1",
        "options": {"viewport": {"width": width, "height": height}},
        "platform": platform,
        "wallTime": wall_time,
        "monotonicTime": monotonic_time,
        "sdkLanguage": "javascript",
        "testIdAttributeName": "data-testid",
        "title": title,
    })
    .to_string()
}

/// Playwright's `renderFullTitleForCall` for the input actions: the title
/// from the protocol's metainfo, then the locator (`{selector}`) when known.
pub fn action_title(method: &str, params: &serde_json::Value, locator: Option<&str>) -> String {
    let param = |k: &str| {
        params
            .get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .replace('\n', "\\n")
    };
    let title = match method {
        "check" => "Check".to_owned(),
        "uncheck" => "Uncheck".to_owned(),
        "click" => "Click".to_owned(),
        "dblclick" => "Double click".to_owned(),
        "hover" => "Hover".to_owned(),
        "tap" => "Tap".to_owned(),
        "fill" => format!("Fill \"{}\"", param("value")),
        "press" => format!("Press \"{}\"", param("key")),
        "type" => format!("Type \"{}\"", param("text")),
        "selectOption" => "Select option".to_owned(),
        "setInputFiles" => "Set input files".to_owned(),
        other => return other.to_owned(),
    };
    match locator {
        Some(l) => format!("{title} {}", l.replace('\n', "\\n")),
        None => title,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sha1_matches_known_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // Playwright 1.63 named this stylesheet resources/3fe3...181a.css.
        assert_eq!(
            sha1_hex(b"button { color: red; }"),
            "3fe33742618fac5890c933e5782ff6af7a2f181a"
        );
        assert_eq!(sha1_hex(&vec![b'a'; 1000]).len(), 40);
    }

    #[test]
    fn action_titles_like_playwright() {
        let p = serde_json::json!({"value": "a\nb", "key": "Enter", "text": "hi"});
        let l = Some("getByRole('button', { name: 'Go' })");
        assert_eq!(
            action_title("click", &p, l),
            "Click getByRole('button', { name: 'Go' })"
        );
        assert_eq!(
            action_title("dblclick", &p, l),
            "Double click getByRole('button', { name: 'Go' })"
        );
        assert_eq!(action_title("fill", &p, Some("#i")), "Fill \"a\\nb\" #i");
        assert_eq!(action_title("press", &p, None), "Press \"Enter\"");
        assert_eq!(action_title("type", &p, None), "Type \"hi\"");
        assert_eq!(action_title("selectOption", &p, None), "Select option");
        assert_eq!(action_title("check", &p, None), "Check");
        assert_eq!(action_title("setInputFiles", &p, None), "Set input files");
        assert_eq!(action_title("mystery", &p, None), "mystery");
    }

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
        // Recorded from Playwright 1.65.1: trace format 10, options without
        // `noDefaultViewport`.
        assert_eq!(v["version"], 10);
        assert_eq!(v["playwrightVersion"], "1.65.1");
        assert_eq!(
            v["options"],
            serde_json::json!({"viewport": {"width": 1280, "height": 720}})
        );
        assert_eq!(v["browserName"], "chromium");
        assert_eq!(v["title"], "t");
    }
}

/// Playwright's protocol metainfo `slowMo: true`: the calls `slowMo` delays.
pub fn slows_down(class: &str, method: &str) -> bool {
    const FRAME: &[&str] = &[
        "blur",
        "check",
        "click",
        "dragAndDrop",
        "drop",
        "dblclick",
        "dispatchEvent",
        "fill",
        "focus",
        "goto",
        "hover",
        "press",
        "selectOption",
        "setInputFiles",
        "tap",
        "type",
        "uncheck",
    ];
    const ELEMENT: &[&str] = &[
        "check",
        "click",
        "dblclick",
        "dispatchEvent",
        "fill",
        "focus",
        "hover",
        "press",
        "scrollIntoViewIfNeeded",
        "selectOption",
        "selectText",
        "setInputFiles",
        "tap",
        "type",
        "uncheck",
    ];
    const PAGE: &[&str] = &[
        "goBack",
        "goForward",
        "reload",
        "keyboardDown",
        "keyboardUp",
        "keyboardInsertText",
        "keyboardType",
        "keyboardPress",
        "mouseMove",
        "mouseDown",
        "mouseUp",
        "mouseClick",
        "mouseWheel",
        "touchscreenTap",
    ];
    match class {
        "Frame" => FRAME.contains(&method),
        "ElementHandle" => ELEMENT.contains(&method),
        "Page" => PAGE.contains(&method),
        _ => false,
    }
}

#[cfg(test)]
mod slow_mo_tests {
    #[test]
    fn actions_and_navigation_slow_down_queries_do_not() {
        assert!(super::slows_down("Frame", "click"));
        assert!(super::slows_down("Frame", "goto"));
        assert!(super::slows_down("Page", "keyboardPress"));
        assert!(!super::slows_down("Frame", "setContent"));
        assert!(!super::slows_down("Frame", "evaluateExpression"));
    }
}
