//! WebMCP (Playwright 1.64's `page.webmcp`): the tools a page registers with
//! `document.modelContext.registerTool()`, as Playwright reports them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool registered by the page (`webmcp.tools()`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebMcpTool {
    pub name: String,
    pub description: String,
    /// JSON schema of the tool's input, when it has one.
    pub input_schema: Option<Value>,
    /// Only the hints that are set (Playwright drops `false` ones).
    pub annotations: Option<WebMcpAnnotations>,
}

/// A tool's hints (`readOnlyHint`, `untrustedContentHint`, `consequentialHint`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebMcpAnnotations {
    pub read_only: bool,
    pub untrusted_content: bool,
    pub consequential: bool,
}

/// Whether the launch arguments enable Chromium's `WebMCP` feature.
pub fn webmcp_enabled(args: &[String]) -> bool {
    args.iter()
        .filter_map(|a| a.strip_prefix("--enable-features="))
        .flat_map(|features| features.split(','))
        .any(|f| f == "WebMCP")
}

/// Playwright's error when WebMCP is used without the feature.
pub const NOT_ENABLED: &str =
    "WebMCP is not enabled. Launch the browser with the \"--enable-features=WebMCP\" argument.";

/// Playwright's `normalizeTool`, from the injected script's description.
pub fn normalize_tool(tool: &Value) -> WebMcpTool {
    let hint = |k: &str| tool.pointer(&format!("/annotations/{k}")) == Some(&Value::Bool(true));
    let annotations = WebMcpAnnotations {
        read_only: hint("readOnly"),
        untrusted_content: hint("untrustedContent"),
        consequential: hint("consequential"),
    };
    let text = |k: &str| {
        tool.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    WebMcpTool {
        name: text("name"),
        description: text("description"),
        input_schema: tool.get("inputSchema").filter(|v| !v.is_null()).cloned(),
        annotations: (annotations != WebMcpAnnotations::default()).then_some(annotations),
    }
}

/// Playwright's error for `callTool` with an unknown name.
pub fn unknown_tool_error(name: &str, available: &[String]) -> String {
    let tail = if available.is_empty() {
        " The frame does not register any WebMCP tools.".to_owned()
    } else {
        format!(" Available tools: {}.", available.join(", "))
    };
    format!("No WebMCP tool named \"{name}\".{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn feature_must_be_in_an_enable_features_argument() {
        assert!(webmcp_enabled(&args(&["--enable-features=WebMCP"])));
        assert!(webmcp_enabled(&args(&[
            "--lang=de",
            "--enable-features=Foo,WebMCP,Bar"
        ])));
        assert!(!webmcp_enabled(&args(&[
            "--enable-features=DeclarativeWebMCP"
        ])));
        assert!(!webmcp_enabled(&args(&["--disable-features=WebMCP"])));
        assert!(!webmcp_enabled(&[]));
    }

    #[test]
    fn tools_are_normalized_like_playwright() {
        // As the injected WebMCPScript describes a tool.
        let raw = json!({"name": "add", "description": "Adds",
            "inputSchema": {"type": "object"},
            "annotations": {"readOnly": true, "untrustedContent": false}});
        assert_eq!(
            normalize_tool(&raw),
            WebMcpTool {
                name: "add".into(),
                description: "Adds".into(),
                input_schema: Some(json!({"type": "object"})),
                annotations: Some(WebMcpAnnotations {
                    read_only: true,
                    ..Default::default()
                }),
            }
        );
        let bare = json!({"name": "t", "description": "", "annotations": {"readOnly": false}});
        assert_eq!(
            normalize_tool(&bare),
            WebMcpTool {
                name: "t".into(),
                description: String::new(),
                input_schema: None,
                annotations: None,
            }
        );
    }

    #[test]
    fn unknown_tool_lists_the_available_ones() {
        assert_eq!(
            unknown_tool_error("nope", &["add".into(), "sub".into()]),
            "No WebMCP tool named \"nope\". Available tools: add, sub."
        );
        assert_eq!(
            unknown_tool_error("nope", &[]),
            "No WebMCP tool named \"nope\". The frame does not register any WebMCP tools."
        );
    }
}
