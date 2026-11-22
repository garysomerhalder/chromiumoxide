//! `ariaSnapshot({ mode, depth, boxes })` and `page.getByRef()`.
//!
//! A port of Playwright's `ariaSnapshotJSONForFrame`: in 'ai' mode the
//! snapshots of child frames are taken through their iframe's ref and merged
//! under the iframe node, then the tree is rendered by Playwright's own
//! `renderAriaSnapshotAsYaml` (from the injected script).

use std::future::Future;
use std::pin::Pin;

use serde_json::{Map, Value, json};

use crate::application::locator::Locator;
use crate::application::page::Page;
use crate::domain::error::Result;

/// `ariaSnapshot`'s `mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AriaSnapshotMode {
    /// Plain YAML for `toMatchAriaSnapshot` (the default).
    #[default]
    Default,
    /// For AI agents: `[ref=e5]` on every node, iframes included; the refs
    /// work with [`Page::get_by_ref`].
    Ai,
}

/// Options of `locator.ariaSnapshot()` / `page.ariaSnapshot()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AriaSnapshotOptions {
    pub mode: AriaSnapshotMode,
    /// Limit the depth of the tree (Playwright's `depth`).
    pub depth: Option<u32>,
    /// Add `[box=x,y,width,height]` to each node.
    pub boxes: bool,
}

impl AriaSnapshotOptions {
    fn mode_name(&self) -> &'static str {
        match self.mode {
            AriaSnapshotMode::Default => "default",
            AriaSnapshotMode::Ai => "ai",
        }
    }
}

/// What the injected `ariaSnapshotJSON` returns.
struct FrameSnapshot {
    json: Vec<Value>,
    iframe_refs: Vec<String>,
    iframe_depths: Map<String, Value>,
    frame: Option<String>,
}

type Boxed<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

impl Locator {
    /// `locator.ariaSnapshot(options)`.
    pub async fn aria_snapshot_with(&self, options: AriaSnapshotOptions) -> Result<String> {
        let json = match options.mode {
            AriaSnapshotMode::Default => {
                let el = self
                    .element("taking an aria snapshot", self.timeout())
                    .await?;
                let v = self
                    .page()
                    .element_call(
                        &el,
                        "(i, node, o) => i.ariaSnapshotJSON(node, o).json",
                        &[json!({"mode": "default", "depth": options.depth, "boxes": options.boxes})],
                        true,
                    )
                    .await?;
                v.as_array().cloned().unwrap_or_default()
            }
            AriaSnapshotMode::Ai => {
                let depth = options.depth.map(i64::from);
                snapshot_for_frame(self, &options, depth, true).await?
            }
        };
        self.page().render_aria_yaml(json).await
    }
}

/// `ariaSnapshotJSONForFrame`: the snapshot of `locator`, with the
/// snapshots of rendered iframes merged in ('ai' mode).
fn snapshot_for_frame<'a>(
    locator: &'a Locator,
    options: &'a AriaSnapshotOptions,
    depth: Option<i64>,
    top: bool,
) -> Boxed<'a, Vec<Value>> {
    Box::pin(async move {
        let Some(snapshot) = frame_snapshot(locator, options, depth, top).await? else {
            return Ok(Vec::new());
        };
        let mut children: Vec<(String, Vec<Value>)> = Vec::new();
        for r in &snapshot.iframe_refs {
            let Some(at) = snapshot.iframe_depths.get(r).and_then(Value::as_i64) else {
                continue;
            };
            // Playwright: `options.depth ? options.depth - iframeDepth - 1 : undefined`.
            let child_depth = depth.filter(|d| *d != 0).map(|d| d - at - 1);
            let child = Locator::in_frame(
                locator.page(),
                snapshot.frame.clone(),
                format!("aria-ref={r} >> internal:control=enter-frame >> body,frameset"),
            );
            let json = snapshot_for_frame(&child, options, child_depth, false)
                .await
                .unwrap_or_default();
            children.push((r.clone(), json));
        }
        let mut json = snapshot.json;
        for node in &mut json {
            merge_iframe_children(node, &children);
        }
        Ok(json)
    })
}

async fn frame_snapshot(
    locator: &Locator,
    options: &AriaSnapshotOptions,
    depth: Option<i64>,
    top: bool,
) -> Result<Option<FrameSnapshot>> {
    let el = if top {
        locator
            .element("taking an aria snapshot", locator.timeout())
            .await?
    } else {
        match locator.query(false, locator.timeout()).await? {
            Some(el) => el,
            None => return Ok(None),
        }
    };
    let frame = locator
        .scope(locator.timeout())
        .await?
        .and_then(|(f, _, _)| f);
    let v = locator
        .page()
        .element_call(
            &el,
            "(i, node, o) => i.ariaSnapshotJSON(node, o)",
            &[json!({"mode": options.mode_name(), "depth": depth, "boxes": options.boxes})],
            true,
        )
        .await?;
    Ok(Some(FrameSnapshot {
        json: v["json"].as_array().cloned().unwrap_or_default(),
        iframe_refs: v["iframeRefs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        iframe_depths: v["iframeDepths"].as_object().cloned().unwrap_or_default(),
        frame,
    }))
}

fn merge_iframe_children(node: &mut Value, children: &[(String, Vec<Value>)]) {
    let Some(obj) = node.as_object_mut() else {
        return;
    };
    if obj.get("role").and_then(Value::as_str) == Some("iframe")
        && let Some(r) = obj.get("ref").and_then(Value::as_str)
    {
        if let Some((_, child)) = children.iter().find(|(k, _)| k == r)
            && !child.is_empty()
        {
            obj.remove("text");
            obj.insert("children".into(), Value::Array(child.clone()));
        }
        return;
    }
    if let Some(Value::Array(kids)) = obj.get_mut("children") {
        for kid in kids {
            merge_iframe_children(kid, children);
        }
    }
}

impl Page {
    /// `page.ariaSnapshot(options)`.
    pub async fn aria_snapshot_with(&self, options: AriaSnapshotOptions) -> Result<String> {
        self.locator("body,frameset")
            .aria_snapshot_with(options)
            .await
    }

    /// `page.getByRef(ref)`: the element with this ref in the last 'ai'
    /// [`aria snapshot`](Page::aria_snapshot_with) (`e5`, or `f1e3` in a frame).
    pub fn get_by_ref(&self, aria_ref: &str) -> Locator {
        self.locator(&format!("aria-ref={aria_ref}"))
    }

    /// Renders snapshot JSON nodes with Playwright's `renderAriaSnapshotAsYaml`.
    async fn render_aria_yaml(&self, json: Vec<Value>) -> Result<String> {
        let v = self
            .injected_call(
                None,
                "(i, json) => i.__chromiumoxide_render_aria_yaml(json)",
                &[Value::Array(json)],
                true,
                self.default_timeout(),
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }
}
