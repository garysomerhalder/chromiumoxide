//! The rest of Playwright's `Page` on Firefox: recent console messages,
//! errors and requests; default timeouts; opener and context; bringToFront;
//! waitForNavigation; pickLocator; pause. And FrameLocator's nth, first,
//! last and owner.

use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::StreamExt;
use serde_json::{Value, json};

use crate::application::debugger::{PICK_STOP, PICK_TICK};
use crate::application::extras::EmulateMedia;
use crate::application::firefox::{BrowserContext, Page, UTILITY};
use crate::application::firefox_events::{ConsoleMessage, PageError, PageEvent};
use crate::application::firefox_locator::{FrameLocator, Locator};
use crate::application::firefox_route::Request;
use crate::application::wait::with_timeout;
use crate::domain::error::{Error, Result};
use crate::domain::selectors::{self, TextMatch, compose};

impl Page {
    /// `page.context()`: the context the page belongs to (`None` once it
    /// is gone).
    pub fn context(&self) -> Option<BrowserContext> {
        self.inner.owner.upgrade().map(BrowserContext::from_inner)
    }

    /// `page.opener()`: the page that opened this one as a pop-up.
    pub fn opener(&self) -> Option<Page> {
        self.inner.opener.lock().unwrap().clone()
    }

    /// `page.bringToFront()`: activates the page's tab.
    pub async fn bring_to_front(&self) -> Result<()> {
        self.bidi_call(
            "browsingContext.activate",
            json!({"context": self.main_context()}),
        )
        .await
        .map(drop)
    }

    /// `page.consoleMessages()`: the most recent console messages (up to 200).
    pub fn console_messages(&self) -> Vec<ConsoleMessage> {
        self.inner
            .history
            .lock()
            .unwrap()
            .console
            .iter()
            .cloned()
            .collect()
    }

    /// `page.pageErrors()`: the most recent uncaught errors (up to 200).
    pub fn page_errors(&self) -> Vec<PageError> {
        self.inner
            .history
            .lock()
            .unwrap()
            .errors
            .iter()
            .cloned()
            .collect()
    }

    /// `page.requests()`: the most recent requests (up to 100).
    pub fn requests(&self) -> Vec<Request> {
        self.inner
            .history
            .lock()
            .unwrap()
            .requests
            .iter()
            .cloned()
            .collect()
    }

    /// `page.clearConsoleMessages()`.
    pub fn clear_console_messages(&self) {
        self.inner.history.lock().unwrap().console.clear();
    }

    /// `page.clearPageErrors()`.
    pub fn clear_page_errors(&self) {
        self.inner.history.lock().unwrap().errors.clear();
    }

    /// `page.setDefaultTimeout(timeout)`: for actions and waits.
    pub fn set_default_timeout(&self, timeout: Duration) {
        self.inner.timeouts.lock().unwrap().action = Some(timeout);
    }

    /// `page.setDefaultNavigationTimeout(timeout)`.
    pub fn set_default_navigation_timeout(&self, timeout: Duration) {
        self.inner.timeouts.lock().unwrap().navigation = Some(timeout);
    }

    /// `page.waitForNavigation()` (deprecated in Playwright: use
    /// `wait_for_url`): the next `load` of the main frame. Listens from the
    /// call on, so start it before the action.
    pub fn wait_for_navigation(&self) -> impl Future<Output = Result<()>> + Send + use<> {
        let mut events = self.events();
        let clock = self.inner.services.clock.clone();
        let timeout = self.navigation_timeout();
        async move {
            with_timeout(&*clock, timeout, "waiting for navigation", async {
                while let Some(e) = events.next().await {
                    match e {
                        PageEvent::Load => return Ok(()),
                        PageEvent::Close => break,
                        _ => {}
                    }
                }
                Err(Error::TargetClosed("page closed".into()))
            })
            .await
        }
    }

    /// `page.pickLocator()`: waits for the user to click an element (in any
    /// frame) and returns its locator. Hovering highlights elements.
    pub async fn pick_locator(&self) -> Result<Locator> {
        self.inner.picking.store(true, Ordering::SeqCst);
        let result = loop {
            if !self.inner.picking.load(Ordering::SeqCst) || self.is_closed() {
                break Err(Error::InvalidArgument(
                    "Locator picking was cancelled".into(),
                ));
            }
            let mut picked = None;
            for frame in self.frames().await.unwrap_or_default() {
                if let Ok(Value::String(selector)) =
                    self.injected_call_in(&frame.context, PICK_TICK, &[]).await
                {
                    picked = Some(if frame.parent.is_none() {
                        self.locator(&selector)
                    } else {
                        frame.locator(&selector)
                    });
                    break;
                }
            }
            if let Some(locator) = picked {
                break Ok(locator);
            }
            self.sleep(Duration::from_millis(100)).await;
        };
        self.inner.picking.store(false, Ordering::SeqCst);
        for frame in self.frames().await.unwrap_or_default() {
            let _ = self
                .call_json_in(&frame.context, PICK_STOP, &[], Some(UTILITY))
                .await;
        }
        result
    }

    /// `page.cancelPickLocator()`: leaves picking mode (no-op otherwise).
    pub async fn cancel_pick_locator(&self) -> Result<()> {
        self.inner.picking.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// `page.pause()`: waits until `context.resume()` (Playwright's
    /// inspector "Resume").
    pub async fn pause(&self) -> Result<()> {
        let Some(ctx) = self.inner.owner.upgrade() else {
            return Ok(());
        };
        ctx.paused.send_replace(true);
        let mut rx = ctx.paused.subscribe();
        while *rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                break;
            }
        }
        Ok(())
    }

    /// `page.requestGC()`: WebDriver BiDi has no garbage-collection command
    /// in Firefox.
    pub async fn request_gc(&self) -> Result<()> {
        Err(Error::InvalidArgument(
            "page.requestGC() is not available in Firefox: WebDriver BiDi has no garbage \
             collection command"
                .into(),
        ))
    }

    /// `page.emulateMedia(...)`: WebDriver BiDi in Firefox cannot emulate
    /// media type, color scheme, reduced motion, forced colors or contrast.
    pub async fn emulate_media(&self, _options: EmulateMedia) -> Result<()> {
        Err(Error::InvalidArgument(
            "page.emulateMedia() is not available in Firefox: WebDriver BiDi has no media \
             emulation command"
                .into(),
        ))
    }
}

impl FrameLocator {
    /// `frameLocator.nth(index)`: the iframe at `index` among the matches.
    pub fn nth(&self, index: i64) -> FrameLocator {
        let mut frames = self.frames.clone();
        if let Some(last) = frames.last_mut() {
            *last = compose::nth(last, index);
        }
        FrameLocator {
            page: self.page.clone(),
            root: self.root.clone(),
            frames,
        }
    }
    pub fn first(&self) -> FrameLocator {
        self.nth(0)
    }
    pub fn last(&self) -> FrameLocator {
        self.nth(-1)
    }

    /// `frameLocator.owner()`: the `<iframe>` element itself.
    pub fn owner(&self) -> Locator {
        let mut frames = self.frames.clone();
        let selector = frames.pop().unwrap_or_default();
        Locator {
            page: self.page.clone(),
            root: self.root.clone(),
            frames,
            selector,
        }
    }

    pub fn get_by_alt_text(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_alt_text(&text.into(), exact))
    }
    pub fn get_by_title(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_title(&text.into(), exact))
    }
}
