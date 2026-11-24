//! Waits on the Firefox engine (`waitForURL`, `waitForLoadState`,
//! `waitForRequest`, `waitForResponse`) and locator extras (`filter`, `and`,
//! `or`, `all`, `boundingBox`, `evaluate`).

use std::future::Future;
use std::time::Duration;

use futures::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::application::firefox::Page;
use crate::application::firefox_events::PageEvent;
use crate::application::firefox_locator::Locator;
use crate::application::firefox_route::{Request, Response};
use crate::application::locator::BoundingBox;
use crate::application::wait::with_timeout;
use crate::domain::error::{Error, Result};
use crate::domain::frame::LifecycleEvent;
use crate::domain::selectors::{Filter, compose};
use crate::domain::url_matcher::UrlMatcher;

/// Playwright's default timeout for waits.
const TIMEOUT: Duration = Duration::from_secs(30);
/// No network activity for this long is `networkidle`.
const IDLE: Duration = Duration::from_millis(500);

impl Page {
    /// `page.waitForURL(url)`: until the main frame's URL matches, then its
    /// `load`.
    pub async fn wait_for_url(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let matcher = url.into();
        let deadline = self.now() + TIMEOUT;
        while !matcher.matches(&self.url()) {
            if self.now() >= deadline {
                return Err(Error::Timeout {
                    ms: TIMEOUT.as_millis() as u64,
                    action: format!("waiting for URL, now {}", self.url()),
                });
            }
            self.sleep(Duration::from_millis(16)).await;
        }
        self.wait_for_load_state(LifecycleEvent::Load).await
    }

    /// `page.waitForLoadState(state)`: `domcontentloaded` and `load` from
    /// the document's `readyState`; `networkidle` is `load` and then no
    /// request, response or finish for 500 ms.
    pub async fn wait_for_load_state(&self, state: LifecycleEvent) -> Result<()> {
        let ready = match state {
            LifecycleEvent::Commit => return Ok(()),
            LifecycleEvent::DomContentLoaded => "document.readyState !== 'loading'",
            LifecycleEvent::Load | LifecycleEvent::NetworkIdle => {
                "document.readyState === 'complete'"
            }
        };
        let mut events = self.events();
        self.wait_for_function(ready).await?;
        if state != LifecycleEvent::NetworkIdle {
            return Ok(());
        }
        let clock = self.inner.services.clock.clone();
        let quiet = async {
            loop {
                let busy = with_timeout(&*clock, IDLE, "network idle", async {
                    loop {
                        match events.next().await {
                            Some(
                                PageEvent::Request(_)
                                | PageEvent::Response(_)
                                | PageEvent::RequestFinished(_)
                                | PageEvent::RequestFailed(_),
                            ) => return Ok(()),
                            Some(PageEvent::Close) | None => {
                                return Err(Error::TargetClosed("page closed".into()));
                            }
                            Some(_) => {}
                        }
                    }
                })
                .await;
                match busy {
                    Ok(()) => continue,
                    Err(Error::Timeout { .. }) => return Ok(()),
                    Err(e) => return Err(e),
                }
            }
        };
        with_timeout(&*clock, TIMEOUT, "waiting for networkidle", quiet).await
    }

    /// `page.waitForRequest(url)`: the next request to a matching URL.
    /// Listens from the call on, so start it before the action.
    pub fn wait_for_request(
        &self,
        url: impl Into<UrlMatcher>,
    ) -> impl Future<Output = Result<Request>> + Send + 'static {
        let matcher = url.into();
        let mut events = self.events();
        let clock = self.inner.services.clock.clone();
        async move {
            with_timeout(&*clock, TIMEOUT, "waiting for request", async {
                while let Some(e) = events.next().await {
                    if let PageEvent::Request(r) = e
                        && matcher.matches(r.url())
                    {
                        return Ok(r);
                    }
                }
                Err(Error::TargetClosed("page closed".into()))
            })
            .await
        }
    }

    /// `page.waitForResponse(url)`: the next response from a matching URL.
    pub fn wait_for_response(
        &self,
        url: impl Into<UrlMatcher>,
    ) -> impl Future<Output = Result<Response>> + Send + 'static {
        let matcher = url.into();
        let mut events = self.events();
        let clock = self.inner.services.clock.clone();
        async move {
            with_timeout(&*clock, TIMEOUT, "waiting for response", async {
                while let Some(e) = events.next().await {
                    if let PageEvent::Response(r) = e
                        && matcher.matches(r.url())
                    {
                        return Ok(r);
                    }
                }
                Err(Error::TargetClosed("page closed".into()))
            })
            .await
        }
    }
}

impl Locator {
    /// `locator.filter({ hasText, hasNotText, has, hasNot, visible })`.
    pub fn filter(&self, filter: Filter) -> Locator {
        self.with(compose::filter(self.selector(), &filter))
    }
    /// `locator.and(other)`: elements matching both.
    pub fn and(&self, other: &Locator) -> Locator {
        self.with(compose::and(self.selector(), other.selector()))
    }
    /// `locator.or(other)`: elements matching either.
    pub fn or(&self, other: &Locator) -> Locator {
        self.with(compose::or(self.selector(), other.selector()))
    }

    /// `locator.all()`: one locator per currently matching element.
    pub async fn all(&self) -> Result<Vec<Locator>> {
        let n = self.count().await?;
        Ok((0..n as i64).map(|i| self.nth(i)).collect())
    }

    /// `locator.boundingBox()`: the element's box in the page's viewport
    /// (CSS px), `None` when it is not rendered.
    pub async fn bounding_box(&self) -> Result<Option<BoundingBox>> {
        let offset = self.frame_offset().await?;
        let v = self
            .read_value(
                "(i, node) => { const r = node.getBoundingClientRect(); \
                 if (!r.width && !r.height && !node.getClientRects().length) return null; \
                 return { x: r.x, y: r.y, width: r.width, height: r.height }; }",
                &[],
            )
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        let n = |k: &str| v[k].as_f64().unwrap_or_default();
        Ok(Some(BoundingBox {
            x: n("x") + offset.0,
            y: n("y") + offset.1,
            width: n("width"),
            height: n("height"),
        }))
    }

    /// `locator.evaluate(fn)`: `fn(element)` (awaited) with a JSON result.
    pub async fn evaluate<T: DeserializeOwned>(&self, function: &str) -> Result<T> {
        self.evaluate_with(function, Value::Null).await
    }

    /// `locator.evaluate(fn, arg)`.
    pub async fn evaluate_with<T: DeserializeOwned, A: Serialize>(
        &self,
        function: &str,
        arg: A,
    ) -> Result<T> {
        let arg = serde_json::to_value(arg)?;
        let v = self
            .read_value(
                &format!("(i, node, arg) => ({function})(node, arg)"),
                &[arg],
            )
            .await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }
}
