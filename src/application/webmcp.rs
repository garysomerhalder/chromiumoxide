//! `page.webmcp` / `frame.webmcp` (Playwright 1.64): the WebMCP tools a
//! frame registers, listed and called through Playwright's own page script.

use serde::Serialize;
use serde_json::{Value, json};

use crate::application::frame::Frame;
use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::frame::World;
use crate::domain::webmcp::{
    NOT_ENABLED, WebMcpTool, normalize_tool, unknown_tool_error, webmcp_enabled,
};

const SOURCE: &str = include_str!("scripts/webmcp.js");

/// The WebMCP tools of one frame. Chromium needs `--enable-features=WebMCP`
/// in [`LaunchOptions::args`](crate::LaunchOptions) and a secure origin.
#[derive(Clone, Debug)]
pub struct WebMcp {
    frame: Frame,
}

impl Page {
    /// `page.webmcp`: the main frame's WebMCP tools.
    pub fn webmcp(&self) -> WebMcp {
        self.main_frame().webmcp()
    }
}

impl Frame {
    /// `frame.webmcp`: the tools this frame registers.
    pub fn webmcp(&self) -> WebMcp {
        WebMcp {
            frame: self.clone(),
        }
    }
}

fn script(call: &str) -> String {
    format!(
        "function(name, input) {{ return (() => {{ const module = {{}};\n{SOURCE}\nreturn new (module.exports.WebMCPScript())(globalThis); }})().{call}; }}"
    )
}

impl WebMcp {
    /// Playwright's `assertBrowserSupport`: a launched Chromium must have the
    /// `WebMCP` feature; a connected browser is not checked.
    fn check_support(&self) -> Result<()> {
        let shared = &self.frame.page().inner.shared;
        match &shared.launch_args {
            Some(args) if !webmcp_enabled(args) => Err(Error::Evaluation(NOT_ENABLED.into())),
            _ => Ok(()),
        }
    }

    async fn run(&self, call: &str, args: &[Value], action: &str) -> Result<Value> {
        let page = self.frame.page();
        page.call_function_in(
            Some(self.frame.id()),
            World::Utility,
            &script(call),
            args,
            page.default_timeout(),
            action,
        )
        .await
    }

    /// `webmcp.tools()`.
    pub async fn tools(&self) -> Result<Vec<WebMcpTool>> {
        self.check_support()?;
        let v = self.run("tools()", &[], "listing WebMCP tools").await?;
        Ok(v.as_array()
            .into_iter()
            .flatten()
            .map(normalize_tool)
            .collect())
    }

    /// `webmcp.callTool(name, input)`: the tool's result.
    pub async fn call_tool(&self, name: &str, input: impl Serialize) -> Result<Value> {
        let tools = self.tools().await?;
        if !tools.iter().any(|t| t.name == name) {
            let available: Vec<String> = tools.into_iter().map(|t| t.name).collect();
            return Err(Error::Evaluation(unknown_tool_error(name, &available)));
        }
        let mut input = serde_json::to_value(input)?;
        if input.is_null() {
            input = json!({});
        }
        self.run(
            "callTool(name, input)",
            &[json!(name), input],
            "calling a WebMCP tool",
        )
        .await
    }
}
