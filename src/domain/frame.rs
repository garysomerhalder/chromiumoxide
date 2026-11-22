//! Frame tree, navigation lifecycle and execution contexts of one page.
//!
//! Follows Playwright's `frames.ts`: every committed document starts with
//! `commit`; `domcontentloaded`/`load` come from the browser; `networkidle`
//! fires when a frame and all its children had no requests for 500 ms. Timers
//! are not run here: the model returns [`Effect`]s and is told when they fire.

use std::collections::{BTreeMap, BTreeSet};

/// Playwright's `waitUntil` states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum LifecycleEvent {
    Commit,
    DomContentLoaded,
    #[default]
    Load,
    NetworkIdle,
}

impl LifecycleEvent {
    /// Parses Playwright's names (`networkidle0` is accepted as an alias).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "commit" => Self::Commit,
            "domcontentloaded" => Self::DomContentLoaded,
            "load" => Self::Load,
            "networkidle" | "networkidle0" => Self::NetworkIdle,
            _ => return None,
        })
    }
}

/// How long a frame must be quiet for `networkidle`.
pub const NETWORK_IDLE_MS: u64 = 500;

/// Side effects the application must perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Call [`FrameTree::network_idle_timer_fired`] with these values after
    /// [`NETWORK_IDLE_MS`]. A newer timer for the same frame supersedes it.
    StartNetworkIdleTimer { frame_id: String, generation: u64 },
}

/// Which JavaScript world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum World {
    /// The page's own world.
    Main,
    /// Our isolated world (helpers that page scripts cannot tamper with).
    Utility,
}

/// One frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frame {
    pub id: String,
    pub parent_id: Option<String>,
    pub url: String,
    pub name: String,
    pub loader_id: Option<String>,
    fired: BTreeSet<LifecycleEvent>,
    inflight: BTreeSet<String>,
    document_request: Option<String>,
    idle_self: bool,
    idle_generation: u64,
    contexts: BTreeMap<WorldKey, i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum WorldKey {
    Main,
    Utility,
}

impl Frame {
    pub fn has(&self, event: LifecycleEvent) -> bool {
        self.fired.contains(&event)
    }
}

/// The frames of a page.
#[derive(Debug, Clone, Default)]
pub struct FrameTree {
    frames: BTreeMap<String, Frame>,
    main: Option<String>,
    generation: u64,
}

impl FrameTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn main_frame(&self) -> Option<&Frame> {
        self.main.as_deref().and_then(|id| self.frames.get(id))
    }

    pub fn frame(&self, id: &str) -> Option<&Frame> {
        self.frames.get(id)
    }

    pub fn frames(&self) -> impl Iterator<Item = &Frame> {
        self.frames.values()
    }

    pub fn children(&self, id: &str) -> Vec<&Frame> {
        self.frames
            .values()
            .filter(|f| f.parent_id.as_deref() == Some(id))
            .collect()
    }

    /// `Page.frameAttached` (or a frame found in `Page.getFrameTree`).
    pub fn attached(&mut self, id: &str, parent_id: Option<&str>) -> Vec<Effect> {
        if self.frames.contains_key(id) {
            return Vec::new();
        }
        let mut frame = Frame {
            id: id.to_owned(),
            parent_id: parent_id.map(str::to_owned),
            ..Frame::default()
        };
        frame.fired.insert(LifecycleEvent::Commit);
        self.frames.insert(id.to_owned(), frame);
        if parent_id.is_none() {
            self.main = Some(id.to_owned());
        }
        self.start_idle_timer(id).into_iter().collect()
    }

    /// `Page.frameNavigated` (new document committed).
    pub fn navigated(
        &mut self,
        id: &str,
        parent_id: Option<&str>,
        url: &str,
        name: &str,
        loader_id: &str,
    ) -> Vec<Effect> {
        let mut effects = self.attached(id, parent_id);
        effects.clear();
        // A new document replaces the frame's children.
        let children: Vec<String> = self.children(id).iter().map(|c| c.id.clone()).collect();
        for child in children {
            self.detached(&child);
        }
        if let Some(f) = self.frames.get_mut(id) {
            f.url = url.to_owned();
            f.name = name.to_owned();
            f.loader_id = Some(loader_id.to_owned());
            if f.inflight.contains(loader_id) {
                f.document_request = Some(loader_id.to_owned());
            }
        }
        effects.extend(self.clear_lifecycle(id));
        effects
    }

    /// `Page.navigatedWithinDocument`.
    pub fn navigated_within_document(&mut self, id: &str, url: &str) {
        if let Some(f) = self.frames.get_mut(id) {
            f.url = url.to_owned();
        }
    }

    /// `Page.frameDetached`: removes the frame and its descendants.
    pub fn detached(&mut self, id: &str) {
        let children: Vec<String> = self.children(id).iter().map(|c| c.id.clone()).collect();
        for child in children {
            self.detached(&child);
        }
        self.frames.remove(id);
        if self.main.as_deref() == Some(id) {
            self.main = None;
        }
        self.recalculate_main(None);
    }

    /// `Page.lifecycleEvent` (Chromium names: `DOMContentLoaded`, `load`, …).
    pub fn lifecycle(&mut self, id: &str, loader_id: &str, name: &str) {
        let event = match name {
            "DOMContentLoaded" => LifecycleEvent::DomContentLoaded,
            "load" => LifecycleEvent::Load,
            _ => return,
        };
        let Some(f) = self.frames.get_mut(id) else {
            return;
        };
        if f.loader_id.as_deref().is_some_and(|l| l != loader_id) {
            return;
        }
        f.fired.insert(event);
        self.recalculate_main(None);
    }

    /// The document was replaced in place (`document.open`, `setContent`).
    pub fn clear_lifecycle(&mut self, id: &str) -> Vec<Effect> {
        let Some(f) = self.frames.get_mut(id) else {
            return Vec::new();
        };
        f.fired.clear();
        let keep = f.document_request.clone();
        f.inflight.retain(|r| Some(r) == keep.as_ref());
        self.stop_idle_timer(id);
        let effects: Vec<Effect> = if self.frames[id].inflight.is_empty() {
            self.start_idle_timer(id).into_iter().collect()
        } else {
            Vec::new()
        };
        self.recalculate_main(Some(id));
        if let Some(f) = self.frames.get_mut(id) {
            f.fired.insert(LifecycleEvent::Commit);
        }
        effects
    }

    /// A network request started in a frame.
    pub fn request_started(
        &mut self,
        frame_id: &str,
        request_id: &str,
        is_document: bool,
    ) -> Vec<Effect> {
        let Some(f) = self.frames.get_mut(frame_id) else {
            return Vec::new();
        };
        if is_document {
            f.document_request = Some(request_id.to_owned());
        }
        f.inflight.insert(request_id.to_owned());
        if f.inflight.len() == 1 {
            self.stop_idle_timer(frame_id);
        }
        Vec::new()
    }

    /// A network request finished or failed.
    pub fn request_finished(&mut self, request_id: &str) -> Vec<Effect> {
        let Some(id) = self
            .frames
            .values()
            .find(|f| f.inflight.contains(request_id))
            .map(|f| f.id.clone())
        else {
            return Vec::new();
        };
        let f = self.frames.get_mut(&id).expect("frame");
        f.inflight.remove(request_id);
        if f.document_request.as_deref() == Some(request_id) {
            f.document_request = None;
        }
        if f.inflight.is_empty() {
            self.start_idle_timer(&id).into_iter().collect()
        } else {
            Vec::new()
        }
    }

    /// A timer from [`Effect::StartNetworkIdleTimer`] elapsed.
    pub fn network_idle_timer_fired(&mut self, frame_id: &str, generation: u64) {
        let Some(f) = self.frames.get_mut(frame_id) else {
            return;
        };
        if f.idle_generation != generation {
            return;
        }
        f.idle_self = true;
        self.recalculate_main(None);
    }

    /// `Runtime.executionContextCreated`.
    pub fn context_created(&mut self, frame_id: &str, world: World, context_id: i64) {
        if let Some(f) = self.frames.get_mut(frame_id) {
            f.contexts.insert(key(world), context_id);
        }
    }

    /// `Runtime.executionContextDestroyed`.
    pub fn context_destroyed(&mut self, context_id: i64) {
        for f in self.frames.values_mut() {
            f.contexts.retain(|_, id| *id != context_id);
        }
    }

    /// `Runtime.executionContextsCleared`.
    pub fn contexts_cleared(&mut self) {
        for f in self.frames.values_mut() {
            f.contexts.clear();
        }
    }

    /// The frame owning an execution context.
    pub fn frame_for_context(&self, context_id: i64) -> Option<&Frame> {
        self.frames
            .values()
            .find(|f| f.contexts.values().any(|id| *id == context_id))
    }

    /// The execution context of `world` in a frame, if it exists right now.
    pub fn context(&self, frame_id: &str, world: World) -> Option<i64> {
        self.frames
            .get(frame_id)?
            .contexts
            .get(&key(world))
            .copied()
    }

    fn start_idle_timer(&mut self, id: &str) -> Option<Effect> {
        self.generation += 1;
        let generation = self.generation;
        let f = self.frames.get_mut(id)?;
        if f.fired.contains(&LifecycleEvent::NetworkIdle) {
            return None;
        }
        f.idle_generation = generation;
        Some(Effect::StartNetworkIdleTimer {
            frame_id: id.to_owned(),
            generation,
        })
    }

    fn stop_idle_timer(&mut self, id: &str) {
        self.generation += 1;
        if let Some(f) = self.frames.get_mut(id) {
            f.idle_generation = self.generation;
            f.idle_self = false;
        }
    }

    fn recalculate_main(&mut self, allow_removing: Option<&str>) {
        if let Some(main) = self.main.clone() {
            self.recalculate(&main, allow_removing);
        }
    }

    fn recalculate(&mut self, id: &str, allow_removing: Option<&str>) {
        let children: Vec<String> = self.children(id).iter().map(|c| c.id.clone()).collect();
        let mut idle = self.frames.get(id).is_some_and(|f| f.idle_self);
        for child in children {
            self.recalculate(&child, allow_removing);
            if !self.frames[&child].has(LifecycleEvent::NetworkIdle) {
                idle = false;
            }
        }
        let Some(f) = self.frames.get_mut(id) else {
            return;
        };
        if idle {
            f.fired.insert(LifecycleEvent::NetworkIdle);
        } else if allow_removing != Some(id) {
            f.fired.remove(&LifecycleEvent::NetworkIdle);
        }
    }
}

fn key(world: World) -> WorldKey {
    match world {
        World::Main => WorldKey::Main,
        World::Utility => WorldKey::Utility,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LifecycleEvent::*;

    fn tree_with_main() -> FrameTree {
        let mut t = FrameTree::new();
        t.attached("M", None);
        t.navigated("M", None, "about:blank", "", "L0");
        t
    }

    fn fire_timers(t: &mut FrameTree, effects: Vec<Effect>) {
        for Effect::StartNetworkIdleTimer {
            frame_id,
            generation,
        } in effects
        {
            t.network_idle_timer_fired(&frame_id, generation);
        }
    }

    #[test]
    fn first_attached_frame_without_parent_is_main() {
        let t = tree_with_main();
        assert_eq!(t.main_frame().unwrap().id, "M");
        assert_eq!(t.main_frame().unwrap().url, "about:blank");
    }

    #[test]
    fn commit_then_browser_lifecycle_events() {
        let mut t = tree_with_main();
        t.navigated("M", None, "https://a.test/", "", "L1");
        let m = t.main_frame().unwrap();
        assert!(m.has(Commit) && !m.has(DomContentLoaded) && !m.has(Load));
        assert_eq!(m.loader_id.as_deref(), Some("L1"));
        t.lifecycle("M", "L1", "init");
        t.lifecycle("M", "L1", "DOMContentLoaded");
        assert!(t.main_frame().unwrap().has(DomContentLoaded));
        t.lifecycle("M", "L1", "load");
        assert!(t.main_frame().unwrap().has(Load));
        // Chromium's own networkIdle is ignored: Playwright computes its own.
        t.lifecycle("M", "L1", "networkIdle");
        assert!(!t.main_frame().unwrap().has(NetworkIdle));
    }

    #[test]
    fn stale_lifecycle_events_of_a_previous_document_are_ignored() {
        let mut t = tree_with_main();
        t.navigated("M", None, "https://a.test/", "", "L1");
        t.lifecycle("M", "L0", "load");
        assert!(!t.main_frame().unwrap().has(Load));
    }

    #[test]
    fn new_document_clears_previous_lifecycle() {
        let mut t = tree_with_main();
        t.lifecycle("M", "L0", "load");
        t.navigated("M", None, "https://b.test/", "", "L2");
        assert!(!t.main_frame().unwrap().has(Load));
    }

    #[test]
    fn network_idle_after_quiet_period() {
        let mut t = FrameTree::new();
        t.attached("M", None);
        let e = t.navigated("M", None, "https://a.test/", "", "L1");
        assert_eq!(e.len(), 1, "commit starts an idle timer");
        let e2 = t.request_started("M", "r1", false);
        assert!(e2.is_empty());
        // the timer from the commit is now stale
        fire_timers(&mut t, e);
        assert!(!t.main_frame().unwrap().has(NetworkIdle));
        let e3 = t.request_finished("r1");
        assert_eq!(e3.len(), 1);
        fire_timers(&mut t, e3);
        assert!(t.main_frame().unwrap().has(NetworkIdle));
    }

    #[test]
    fn main_frame_network_idle_waits_for_children() {
        let mut t = FrameTree::new();
        t.attached("M", None);
        let em = t.navigated("M", None, "https://a.test/", "", "L1");
        t.attached("C", Some("M"));
        let ec = t.navigated("C", Some("M"), "https://c.test/", "c", "LC");
        let busy = t.request_started("C", "rc", false);
        assert!(busy.is_empty());
        fire_timers(&mut t, em);
        assert!(!t.main_frame().unwrap().has(NetworkIdle), "child is busy");
        fire_timers(&mut t, ec);
        let done = t.request_finished("rc");
        fire_timers(&mut t, done);
        assert!(t.frame("C").unwrap().has(NetworkIdle));
        assert!(t.main_frame().unwrap().has(NetworkIdle));
        assert_eq!(t.children("M").len(), 1);
    }

    #[test]
    fn document_request_survives_commit_and_counts_as_inflight() {
        let mut t = tree_with_main();
        t.request_started("M", "L5", true);
        let e = t.navigated("M", None, "https://a.test/", "", "L5");
        assert!(e.is_empty(), "document request still in flight");
        let e = t.request_finished("L5");
        fire_timers(&mut t, e);
        assert!(t.main_frame().unwrap().has(NetworkIdle));
    }

    #[test]
    fn clear_lifecycle_restarts_a_document_in_place() {
        let mut t = tree_with_main();
        t.lifecycle("M", "L0", "DOMContentLoaded");
        t.lifecycle("M", "L0", "load");
        let e = t.clear_lifecycle("M");
        let m = t.main_frame().unwrap();
        assert!(m.has(Commit) && !m.has(Load) && !m.has(DomContentLoaded));
        assert_eq!(e.len(), 1);
        t.lifecycle("M", "L0", "load");
        assert!(t.main_frame().unwrap().has(Load));
    }

    #[test]
    fn same_document_navigation_updates_url_only() {
        let mut t = tree_with_main();
        t.lifecycle("M", "L0", "load");
        t.navigated_within_document("M", "about:blank#x");
        let m = t.main_frame().unwrap();
        assert_eq!(m.url, "about:blank#x");
        assert!(m.has(Load));
    }

    #[test]
    fn detaching_removes_descendants() {
        let mut t = tree_with_main();
        t.attached("C", Some("M"));
        t.attached("G", Some("C"));
        t.detached("C");
        assert!(t.frame("C").is_none() && t.frame("G").is_none());
        assert!(t.children("M").is_empty());
    }

    #[test]
    fn execution_contexts_per_world() {
        let mut t = tree_with_main();
        t.context_created("M", World::Main, 1);
        t.context_created("M", World::Utility, 2);
        assert_eq!(t.context("M", World::Main), Some(1));
        assert_eq!(t.context("M", World::Utility), Some(2));
        t.context_destroyed(1);
        assert_eq!(t.context("M", World::Main), None);
        assert_eq!(t.context("M", World::Utility), Some(2));
        assert_eq!(t.frame_for_context(2).map(|f| f.id.as_str()), Some("M"));
        assert!(t.frame_for_context(99).is_none());
        t.contexts_cleared();
        assert_eq!(t.context("M", World::Utility), None);
    }

    #[test]
    fn parse_wait_until() {
        assert_eq!(LifecycleEvent::parse("networkidle0"), Some(NetworkIdle));
        assert_eq!(
            LifecycleEvent::parse("domcontentloaded"),
            Some(DomContentLoaded)
        );
        assert_eq!(LifecycleEvent::parse("nope"), None);
    }
}
