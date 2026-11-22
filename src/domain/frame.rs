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
        unimplemented!()
    }

    pub fn frame(&self, _id: &str) -> Option<&Frame> {
        unimplemented!()
    }

    pub fn children(&self, _id: &str) -> Vec<&Frame> {
        unimplemented!()
    }

    /// `Page.frameAttached` (or a frame found in `Page.getFrameTree`).
    pub fn attached(&mut self, _id: &str, _parent_id: Option<&str>) -> Vec<Effect> {
        unimplemented!()
    }

    /// `Page.frameNavigated` (new document committed).
    pub fn navigated(
        &mut self,
        _id: &str,
        _parent_id: Option<&str>,
        _url: &str,
        _name: &str,
        _loader_id: &str,
    ) -> Vec<Effect> {
        unimplemented!()
    }

    /// `Page.navigatedWithinDocument`.
    pub fn navigated_within_document(&mut self, _id: &str, _url: &str) {
        unimplemented!()
    }

    /// `Page.frameDetached`: removes the frame and its descendants.
    pub fn detached(&mut self, _id: &str) {
        unimplemented!()
    }

    /// `Page.lifecycleEvent` (Chromium names: `DOMContentLoaded`, `load`, …).
    pub fn lifecycle(&mut self, _id: &str, _loader_id: &str, _name: &str) {
        unimplemented!()
    }

    /// The document was replaced in place (`document.open`, `setContent`).
    pub fn clear_lifecycle(&mut self, _id: &str) -> Vec<Effect> {
        unimplemented!()
    }

    /// A network request started in a frame.
    pub fn request_started(
        &mut self,
        _frame_id: &str,
        _request_id: &str,
        _is_document: bool,
    ) -> Vec<Effect> {
        unimplemented!()
    }

    /// A network request finished or failed.
    pub fn request_finished(&mut self, _request_id: &str) -> Vec<Effect> {
        unimplemented!()
    }

    /// A timer from [`Effect::StartNetworkIdleTimer`] elapsed.
    pub fn network_idle_timer_fired(&mut self, _frame_id: &str, _generation: u64) {
        unimplemented!()
    }

    /// `Runtime.executionContextCreated`.
    pub fn context_created(&mut self, _frame_id: &str, _world: World, _context_id: i64) {
        unimplemented!()
    }

    /// `Runtime.executionContextDestroyed`.
    pub fn context_destroyed(&mut self, _context_id: i64) {
        unimplemented!()
    }

    /// `Runtime.executionContextsCleared`.
    pub fn contexts_cleared(&mut self) {
        unimplemented!()
    }

    /// The execution context of `world` in a frame, if it exists right now.
    pub fn context(&self, _frame_id: &str, _world: World) -> Option<i64> {
        unimplemented!()
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
