//! ARIA snapshots on the WebKit engine (`ariaSnapshot`, `ariaSnapshotJSON`,
//! 'ai' mode with refs merged in from iframes) and `page.getByRef()`, from
//! Playwright's injected script, as on Chromium.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use crate::application::aria_snapshot::{
    AriaSnapshotMode, AriaSnapshotOptions, merge_iframe_children,
};
use crate::application::webkit::Page;
use crate::application::webkit_locator::Locator;
use crate::domain::error::Result;

type Boxed<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

impl Locator {
    /// `locator.ariaSnapshot()`: the accessibility tree as YAML.
    pub async fn aria_snapshot(&self) -> Result<String> {
        let v = self
            .read(
                "(i, node) => i.ariaSnapshot(node, { mode: 'default' })",
                &[],
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `locator.ariaSnapshotJSON()`: the accessibility tree as JSON nodes.
    pub async fn aria_snapshot_json(&self) -> Result<Value> {
        self.read(
            "(i, node) => i.ariaSnapshotJSON(node, { mode: 'default' }).json",
            &[],
        )
        .await
    }

    /// `locator.ariaSnapshot({ mode, depth, boxes })`.
    pub async fn aria_snapshot_with(&self, options: AriaSnapshotOptions) -> Result<String> {
        let json = match options.mode {
            AriaSnapshotMode::Default => {
                let v = self
                    .read(
                        "(i, node, o) => i.ariaSnapshotJSON(node, o).json",
                        &[json!({"mode": "default", "depth": options.depth, "boxes": options.boxes})],
                    )
                    .await?;
                v.as_array().cloned().unwrap_or_default()
            }
            AriaSnapshotMode::Ai => {
                let depth = options.depth.map(i64::from);
                snapshot_for_frame(self, &options, depth, true).await?
            }
        };
        let v = self
            .page
            .injected_call_in(
                None,
                "(i, json) => i.__chromiumoxide_render_aria_yaml(json)",
                &[Value::Array(json)],
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }
}

/// Playwright's `ariaSnapshotJSONForFrame`: the snapshot of `locator`, with
/// the snapshots of rendered iframes merged in ('ai' mode).
fn snapshot_for_frame<'a>(
    locator: &'a Locator,
    options: &'a AriaSnapshotOptions,
    depth: Option<i64>,
    top: bool,
) -> Boxed<'a, Vec<Value>> {
    Box::pin(async move {
        let o = json!({"mode": options.mode_name(), "depth": depth, "boxes": options.boxes});
        let v = if top {
            locator
                .read("(i, node, o) => i.ariaSnapshotJSON(node, o)", &[o])
                .await?
        } else {
            let found = locator
                .call(
                    "(i, selector, o) => { const el = i.querySelector(i.parseSelector(selector), document, false); \
                     return el ? i.ariaSnapshotJSON(el, o) : null; }",
                    &[json!(locator.selector), o],
                )
                .await?;
            if found.is_null() || found.get("__frame_missing").is_some() {
                return Ok(Vec::new());
            }
            found
        };
        let mut children: Vec<(String, Vec<Value>)> = Vec::new();
        for r in v["iframeRefs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let Some(at) = v["iframeDepths"].get(r).and_then(Value::as_i64) else {
                continue;
            };
            // Playwright: `options.depth ? options.depth - iframeDepth - 1 : undefined`.
            let child_depth = depth.filter(|d| *d != 0).map(|d| d - at - 1);
            let mut frames = locator.frames.clone();
            frames.push(format!("aria-ref={r}"));
            let child = Locator {
                page: locator.page.clone(),
                root: locator.root.clone(),
                frames,
                selector: "body,frameset".into(),
            };
            let json = snapshot_for_frame(&child, options, child_depth, false)
                .await
                .unwrap_or_default();
            children.push((r.to_owned(), json));
        }
        let mut json = v["json"].as_array().cloned().unwrap_or_default();
        for node in &mut json {
            merge_iframe_children(node, &children);
        }
        Ok(json)
    })
}

impl Page {
    /// `page.ariaSnapshot()`.
    pub async fn aria_snapshot(&self) -> Result<String> {
        self.locator("body,frameset").aria_snapshot().await
    }

    /// `page.ariaSnapshotJSON()`.
    pub async fn aria_snapshot_json(&self) -> Result<Value> {
        self.locator("body,frameset").aria_snapshot_json().await
    }

    /// `page.ariaSnapshot({ mode, depth, boxes })`.
    pub async fn aria_snapshot_with(&self, options: AriaSnapshotOptions) -> Result<String> {
        self.locator("body,frameset")
            .aria_snapshot_with(options)
            .await
    }

    /// `page.getByRef(ref)`: the element with this ref in the last 'ai'
    /// snapshot (`e5`, or `f1e3` in a frame).
    pub fn get_by_ref(&self, aria_ref: &str) -> Locator {
        self.locator(&format!("aria-ref={aria_ref}"))
    }
}
