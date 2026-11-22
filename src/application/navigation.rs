//! Use cases: reload, history navigation, waitForURL, waitForTimeout.

use std::future::IntoFuture;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::application::page::{Page, Response};
use crate::application::wait::{wait_until, with_timeout};
use crate::domain::error::{Error, Result};
use crate::domain::frame::LifecycleEvent;
use crate::domain::url_matcher::UrlMatcher;

/// Which navigation a [`Navigation`] performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Reload,
    Back,
    Forward,
}

impl Page {
    /// Playwright's `page.reload()`.
    pub fn reload(&self) -> Navigation<'_> {
        Navigation::new(self, Kind::Reload)
    }

    /// Playwright's `page.goBack()`: `None` when there is no previous entry.
    pub fn go_back(&self) -> Navigation<'_> {
        Navigation::new(self, Kind::Back)
    }

    /// Playwright's `page.goForward()`: `None` when there is no next entry.
    pub fn go_forward(&self) -> Navigation<'_> {
        Navigation::new(self, Kind::Forward)
    }

    /// Playwright's `page.waitForURL(url)`.
    pub fn wait_for_url(&self, url: impl Into<UrlMatcher>) -> WaitForUrl<'_> {
        WaitForUrl {
            page: self,
            matcher: url.into(),
            wait_until: LifecycleEvent::Load,
            timeout: None,
        }
    }

    /// Playwright's `page.waitForTimeout(ms)` (prefer real waits in tests).
    pub async fn wait_for_timeout(&self, duration: Duration) {
        self.inner.shared.services.clock.sleep(duration).await;
    }

    async fn run_navigation(
        &self,
        kind: Kind,
        wait: LifecycleEvent,
        timeout: Duration,
    ) -> Result<Option<Response>> {
        let clock = &*self.inner.shared.services.clock;
        let action = format!("{kind:?} navigation, waiting until \"{wait:?}\"");
        let before = self.inner.state.lock().unwrap().main_navigations;
        match kind {
            Kind::Reload => {
                with_timeout(clock, timeout, &action, self.send("Page.reload", json!({}))).await?;
            }
            Kind::Back | Kind::Forward => {
                let h = self.send("Page.getNavigationHistory", json!({})).await?;
                let current = h
                    .get("currentIndex")
                    .and_then(Value::as_i64)
                    .unwrap_or_default();
                let target = if kind == Kind::Back {
                    current - 1
                } else {
                    current + 1
                };
                let entries = h
                    .get("entries")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let Some(entry) = usize::try_from(target).ok().and_then(|i| entries.get(i)) else {
                    return Ok(None);
                };
                let id = entry.get("id").cloned().unwrap_or(Value::Null);
                with_timeout(
                    clock,
                    timeout,
                    &action,
                    self.send("Page.navigateToHistoryEntry", json!({"entryId": id})),
                )
                .await?;
            }
        }
        let inner = &self.inner;
        wait_until(clock, &inner.changed, timeout, &action, || {
            if inner.closed.load(Ordering::SeqCst) {
                return Some(Err(Error::TargetClosed("page has been closed".into())));
            }
            let st = inner.state.lock().unwrap();
            if st.main_navigations == before {
                return None;
            }
            if st.last_navigation_same_document {
                return Some(Ok(None));
            }
            let main = st.tree.main_frame()?;
            if !main.has(wait) {
                return None;
            }
            Some(Ok(main
                .loader_id
                .as_ref()
                .and_then(|l| st.responses.get(l))
                .cloned()))
        })
        .await
    }

    async fn run_wait_for_url(
        &self,
        matcher: &UrlMatcher,
        wait: LifecycleEvent,
        timeout: Duration,
    ) -> Result<()> {
        let inner = &self.inner;
        wait_until(
            &*inner.shared.services.clock,
            &inner.changed,
            timeout,
            "waiting for URL",
            || {
                if inner.closed.load(Ordering::SeqCst) {
                    return Some(Err(Error::TargetClosed("page has been closed".into())));
                }
                let st = inner.state.lock().unwrap();
                let main = st.tree.main_frame()?;
                (matcher.matches(&main.url) && main.has(wait)).then_some(Ok(()))
            },
        )
        .await
    }
}

/// A pending reload / back / forward.
#[must_use = "does nothing until awaited"]
pub struct Navigation<'a> {
    page: &'a Page,
    kind: Kind,
    wait_until: LifecycleEvent,
    timeout: Option<Duration>,
}

impl<'a> Navigation<'a> {
    fn new(page: &'a Page, kind: Kind) -> Self {
        Self {
            page,
            kind,
            wait_until: LifecycleEvent::Load,
            timeout: None,
        }
    }
    pub fn wait_until(mut self, event: LifecycleEvent) -> Self {
        self.wait_until = event;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl<'a> IntoFuture for Navigation<'a> {
    type Output = Result<Option<Response>>;
    type IntoFuture = BoxFuture<'a, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let timeout = self.timeout.unwrap_or_else(|| self.page.default_timeout());
            self.page
                .run_navigation(self.kind, self.wait_until, timeout)
                .await
        }
        .boxed()
    }
}

/// A pending `page.waitForURL()`.
#[must_use = "does nothing until awaited"]
pub struct WaitForUrl<'a> {
    page: &'a Page,
    matcher: UrlMatcher,
    wait_until: LifecycleEvent,
    timeout: Option<Duration>,
}

impl<'a> WaitForUrl<'a> {
    pub fn wait_until(mut self, event: LifecycleEvent) -> Self {
        self.wait_until = event;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl<'a> IntoFuture for WaitForUrl<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let timeout = self.timeout.unwrap_or_else(|| self.page.default_timeout());
            self.page
                .run_wait_for_url(&self.matcher, self.wait_until, timeout)
                .await
        }
        .boxed()
    }
}
