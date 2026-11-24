//! Safari over WebDriver (`safaridriver`, Apple's WebDriver server): the
//! server's arguments and the session's capabilities. The same code drives
//! any W3C WebDriver server, such as WebKitGTK's `WebKitWebDriver`.

use std::path::Path;

use serde_json::Value;

/// Where macOS keeps Apple's WebDriver server.
pub const SAFARIDRIVER: &str = "/usr/bin/safaridriver";

/// The server's arguments to listen on `port`: `safaridriver --port N`,
/// `WebKitWebDriver --port=N`.
pub fn driver_args(driver: &Path, port: u16) -> Vec<String> {
    let name = driver
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.starts_with("safaridriver") {
        vec!["--port".into(), port.to_string()]
    } else {
        vec![format!("--port={port}")]
    }
}

/// The `POST /session` body: Safari by default, or `always_match` as given.
pub fn new_session(always_match: Option<&Value>) -> Value {
    let always = always_match
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"browserName": "safari"}));
    serde_json::json!({"capabilities": {"alwaysMatch": always}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn driver_arguments() {
        assert_eq!(
            driver_args(Path::new("/usr/bin/safaridriver"), 4444),
            ["--port", "4444"]
        );
        assert_eq!(
            driver_args(Path::new("/usr/bin/WebKitWebDriver"), 4444),
            ["--port=4444"]
        );
    }

    #[test]
    fn a_tauri_app_on_macos_embeds_its_webdriver_server() {
        // The app is the server: its port goes in TAURI_WEBDRIVER_PORT
        // (tauri-plugin-wdio-webdriver), not in arguments.
        assert_eq!(
            embedded_port_env(4445),
            ("TAURI_WEBDRIVER_PORT".to_owned(), "4445".to_owned())
        );
        assert_eq!(
            embedded_new_session(),
            json!({"capabilities": {"alwaysMatch": {}}})
        );
    }

    #[test]
    fn session_capabilities() {
        assert_eq!(
            new_session(None),
            json!({"capabilities": {"alwaysMatch": {"browserName": "safari"}}})
        );
        let gtk = json!({"webkitgtk:browserOptions": {"binary": "/b", "args": ["--automation"]}});
        assert_eq!(
            new_session(Some(&gtk)),
            json!({"capabilities": {"alwaysMatch": gtk}})
        );
    }
}
