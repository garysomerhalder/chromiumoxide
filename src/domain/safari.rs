//! Safari over WebDriver (`safaridriver`, Apple's WebDriver server): the
//! server's arguments and the session's capabilities. The same code drives
//! any W3C WebDriver server, such as WebKitGTK's `WebKitWebDriver`.

use std::path::Path;

use serde_json::Value;

/// Where macOS keeps Apple's WebDriver server.
pub const SAFARIDRIVER: &str = "/usr/bin/safaridriver";

/// The server's arguments to listen on `port`: `safaridriver --port N`,
/// `WebKitWebDriver --port=N`.
pub fn driver_args(_driver: &Path, _port: u16) -> Vec<String> {
    Vec::new()
}

/// The `POST /session` body: Safari by default, or `always_match` as given.
pub fn new_session(_always_match: Option<&Value>) -> Value {
    Value::Null
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
