//! Out-of-process iframes (site isolation): each cross-site iframe is its own
//! target with its own CDP session. Like Playwright's per-frame sessions, the
//! page routes frame-scoped commands to the right session.
//!
//! The rest of the page code is unaware of these sessions: execution context
//! ids from a child session are tagged with its number (`n * 2^32 + id`) and
//! remote object ids with an `oopif<n>:` prefix, so `Page::send` can route a
//! command by the ids it carries, or by its `frameId`.

use serde_json::{Value, json};

use crate::application::page::{Page, UTILITY_WORLD, load_frame_tree};
use crate::domain::error::Result;
use crate::ports::RawEvent;

const CTX_BASE: i64 = 1 << 32;

/// A child session: its number is its index + 1 (0 is the page's own).
#[derive(Debug, Clone)]
pub(crate) struct Oopif {
    pub(crate) session_id: String,
    /// The iframe's frame id (= its target id).
    pub(crate) frame_id: String,
}

fn prefix(no: usize) -> String {
    format!("oopif{no}:")
}

/// Removes the session tags from outgoing params; the session they name.
fn untag(v: &mut Value, no: &mut usize) {
    match v {
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                match (k.as_str(), &*val) {
                    ("executionContextId" | "contextId", Value::Number(n)) => {
                        if let Some(id) = n.as_i64().filter(|id| *id >= CTX_BASE) {
                            *no = (id / CTX_BASE) as usize;
                            *val = json!(id % CTX_BASE);
                        }
                    }
                    ("objectId" | "requestId", Value::String(s)) => {
                        if let Some(rest) = s.strip_prefix("oopif")
                            && let Some((n, id)) = rest.split_once(':')
                            && let Ok(n) = n.parse::<usize>()
                        {
                            *no = n;
                            *val = json!(id);
                        }
                    }
                    _ => untag(val, no),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                untag(item, no);
            }
        }
        _ => {}
    }
}

/// Tags ids in results and events of child session `no`.
fn tag(v: &mut Value, no: usize) {
    match v {
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                match (k.as_str(), &*val) {
                    ("executionContextId", Value::Number(n)) => {
                        if let Some(id) = n.as_i64() {
                            *val = json!(no as i64 * CTX_BASE + id);
                        }
                    }
                    ("objectId" | "requestId" | "networkId" | "loaderId", Value::String(s))
                        if !s.is_empty() =>
                    {
                        *val = json!(format!("{}{s}", prefix(no)));
                    }
                    _ => tag(val, no),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                tag(item, no);
            }
        }
        _ => {}
    }
}

/// Target-wide setup that every session of the page needs (init scripts,
/// bindings, interception, headers): broadcast and replayed on new iframes.
pub(crate) const BROADCAST: &[&str] = &[
    "Page.addScriptToEvaluateOnNewDocument",
    "Runtime.addBinding",
    "Fetch.enable",
    "Fetch.disable",
    "Network.setExtraHTTPHeaders",
    "Network.setCacheDisabled",
    "Network.setUserAgentOverride",
    "Emulation.setUserAgentOverride",
];

impl Page {
    /// Records a setup command for iframes attached later.
    pub(crate) fn record_setup(&self, method: &str, params: &Value) {
        if BROADCAST.contains(&method) {
            self.inner
                .setup_log
                .lock()
                .unwrap()
                .push((method.to_owned(), params.clone()));
        }
    }

    /// Sends a setup command to every iframe session too.
    pub(crate) async fn broadcast_setup(&self, method: &str, params: &Value) {
        let sessions: Vec<String> = self
            .inner
            .oopifs
            .lock()
            .unwrap()
            .iter()
            .map(|o| o.session_id.clone())
            .collect();
        for sid in sessions {
            let _ = self
                .cdp_session()
                .call(Some(&sid), method, params.clone())
                .await;
        }
    }

    pub(crate) fn oopif_no(&self, session_id: &str) -> Option<usize> {
        self.inner
            .oopifs
            .lock()
            .unwrap()
            .iter()
            .position(|o| o.session_id == session_id)
            .map(|i| i + 1)
    }

    pub(crate) fn has_session(&self, session_id: &str) -> bool {
        self.inner.session_id == session_id || self.oopif_no(session_id).is_some()
    }

    fn session_of_no(&self, no: usize) -> Option<String> {
        if no == 0 {
            return Some(self.inner.session_id.clone());
        }
        self.inner
            .oopifs
            .lock()
            .unwrap()
            .get(no - 1)
            .map(|o| o.session_id.clone())
    }

    /// The session (number) a frame's documents live in.
    pub(crate) fn session_no_for_frame(&self, frame_id: &str) -> usize {
        let roots: Vec<String> = self
            .inner
            .oopifs
            .lock()
            .unwrap()
            .iter()
            .map(|o| o.frame_id.clone())
            .collect();
        if roots.is_empty() {
            return 0;
        }
        let st = self.inner.state.lock().unwrap();
        let mut id = Some(frame_id.to_owned());
        while let Some(f) = id {
            if let Some(i) = roots.iter().rposition(|r| *r == f) {
                return i + 1;
            }
            id = st.tree.frame(&f).and_then(|fr| fr.parent_id.clone());
        }
        0
    }

    fn parent_frame_id(&self, frame_id: &str) -> Option<String> {
        let st = self.inner.state.lock().unwrap();
        st.tree.frame(frame_id).and_then(|f| f.parent_id.clone())
    }

    /// Picks the session for a command, untagging its ids.
    pub(crate) fn route_session(&self, method: &str, params: &mut Value) -> usize {
        let mut no = 0;
        untag(params, &mut no);
        if no == 0
            && let Some(frame) = params.get("frameId").and_then(Value::as_str)
        {
            no = if method == "DOM.getFrameOwner" {
                self.parent_frame_id(frame)
                    .map_or(0, |p| self.session_no_for_frame(&p))
            } else {
                self.session_no_for_frame(frame)
            };
        }
        no
    }

    /// Sends to child session `no`, tagging the result.
    pub(crate) async fn send_in_session(
        &self,
        no: usize,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let Some(sid) = self.session_of_no(no) else {
            return Err(crate::domain::error::Error::TargetClosed(
                "frame has been detached".into(),
            ));
        };
        let mut r = self.cdp_session().call(Some(&sid), method, params).await?;
        tag(&mut r, no);
        if matches!(method, "DOM.getContentQuads" | "DOM.getBoxModel") {
            let (dx, dy) = self.oopif_offset(no).await;
            offset_quads(&mut r, dx, dy);
        }
        Ok(r)
    }

    /// Where child session `no`'s viewport starts, in page coordinates: the
    /// content box of its `<iframe>`.
    fn oopif_offset(&self, no: usize) -> futures::future::BoxFuture<'_, (f64, f64)> {
        Box::pin(async move {
            let Some(frame_id) = self
                .inner
                .oopifs
                .lock()
                .unwrap()
                .get(no - 1)
                .map(|o| o.frame_id.clone())
            else {
                return (0.0, 0.0);
            };
            let parent_no = self
                .parent_frame_id(&frame_id)
                .map_or(0, |p| self.session_no_for_frame(&p));
            let Some(parent_sid) = self.session_of_no(parent_no) else {
                return (0.0, 0.0);
            };
            let cdp = self.cdp_session();
            let Ok(owner) = cdp
                .call(
                    Some(&parent_sid),
                    "DOM.getFrameOwner",
                    json!({"frameId": frame_id}),
                )
                .await
            else {
                return (0.0, 0.0);
            };
            let Ok(model) = cdp
                .call(
                    Some(&parent_sid),
                    "DOM.getBoxModel",
                    json!({"backendNodeId": owner.get("backendNodeId")}),
                )
                .await
            else {
                return (0.0, 0.0);
            };
            let content = &model["model"]["content"];
            let x = content[0].as_f64().unwrap_or_default();
            let y = content[1].as_f64().unwrap_or_default();
            let (px, py) = if parent_no == 0 {
                (0.0, 0.0)
            } else {
                self.oopif_offset(parent_no).await
            };
            (x + px, y + py)
        })
    }

    /// A child session event, with tagged ids; `None` when handled here.
    pub(crate) fn oopif_event(&self, no: usize, e: &RawEvent) -> Option<RawEvent> {
        if e.method == "Runtime.executionContextsCleared" {
            let frames: Vec<String> = {
                let st = self.inner.state.lock().unwrap();
                st.tree.frames().map(|f| f.id.clone()).collect()
            };
            let owned: Vec<String> = frames
                .into_iter()
                .filter(|f| self.session_no_for_frame(f) == no)
                .collect();
            let mut st = self.inner.state.lock().unwrap();
            for f in owned {
                st.tree.contexts_cleared_in(&f);
            }
            return None;
        }
        // Document events of the iframe are not the page's.
        if matches!(
            e.method.as_str(),
            "Page.domContentEventFired" | "Page.loadEventFired" | "Inspector.targetCrashed"
        ) {
            return None;
        }
        let mut e = e.clone();
        if e.method == "Runtime.executionContextCreated" {
            if let Some(id) = e.params["context"]["id"].as_i64() {
                e.params["context"]["id"] = json!(no as i64 * CTX_BASE + id);
            }
        } else {
            tag(&mut e.params, no);
        }
        Some(e)
    }

    /// `Target.attachedToTarget` for an `iframe` target of this page.
    pub(crate) async fn attach_oopif(&self, session_id: String, frame_id: String) {
        let no = {
            let mut list = self.inner.oopifs.lock().unwrap();
            list.push(Oopif {
                session_id: session_id.clone(),
                frame_id: frame_id.clone(),
            });
            list.len()
        };
        let cdp = self.inner.shared.cdp.clone();
        let sid = Some(session_id.as_str());
        // The target waits for the debugger, so its tree is stable until resumed.
        if let Ok(tree) = cdp.call(sid, "Page.getFrameTree", json!({})).await {
            let node = &tree["frameTree"];
            let parent = node["frame"]
                .get("parentId")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| self.parent_frame_id(&frame_id));
            let mut effects = Vec::new();
            {
                let mut st = self.inner.state.lock().unwrap();
                load_frame_tree(&mut st.tree, node, parent.as_deref(), &mut effects);
            }
            self.apply_effects(effects);
        }
        let setup: Vec<(String, Value)> = self.inner.setup_log.lock().unwrap().clone();
        for (method, params) in setup {
            let _ = cdp.call(sid, &method, params).await;
        }
        let _ = futures::future::join_all([
            cdp.call(sid, "Network.enable", json!({})),
            cdp.call(sid, "Page.enable", json!({})),
            cdp.call(
                sid,
                "Page.setLifecycleEventsEnabled",
                json!({"enabled": true}),
            ),
            cdp.call(sid, "Runtime.enable", json!({})),
            cdp.call(
                sid,
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": "", "worldName": UTILITY_WORLD}),
            ),
            cdp.call(
                sid,
                "Target.setAutoAttach",
                json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}),
            ),
            cdp.call(sid, "Runtime.runIfWaitingForDebugger", json!({})),
        ])
        .await;
        let frames: Vec<String> = {
            let st = self.inner.state.lock().unwrap();
            st.tree.frames().map(|f| f.id.clone()).collect()
        };
        for f in frames
            .into_iter()
            .filter(|f| self.session_no_for_frame(f) == no)
        {
            let _ = cdp
                .call(
                    sid,
                    "Page.createIsolatedWorld",
                    json!({"frameId": f, "grantUniveralAccess": true, "worldName": UTILITY_WORLD}),
                )
                .await;
        }
        self.bump();
    }
}

fn offset_quads(r: &mut Value, dx: f64, dy: f64) {
    let shift = |q: &mut Value| {
        if let Some(a) = q.as_array_mut() {
            for (i, v) in a.iter_mut().enumerate() {
                if let Some(n) = v.as_f64() {
                    *v = json!(n + if i % 2 == 0 { dx } else { dy });
                }
            }
        }
    };
    if let Some(quads) = r.get_mut("quads").and_then(Value::as_array_mut) {
        for q in quads {
            shift(q);
        }
    }
    if let Some(model) = r.get_mut("model") {
        for k in ["content", "padding", "border", "margin"] {
            if let Some(q) = model.get_mut(k) {
                shift(q);
            }
        }
    }
}
