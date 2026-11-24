//! The rest of Playwright's `BrowserContext` and `Browser` on Firefox:
//! settings changed after creation (extra headers, offline, geolocation,
//! permissions, HTTP credentials, storage state), context events, and the
//! Chromium-only methods, which return an error that says so.

use std::sync::atomic::Ordering;

use futures::StreamExt;
use serde_json::{Value, json};

use crate::application::firefox::{Browser, BrowserContext, Page};
use crate::application::firefox_events::ContextEvent;
use crate::application::firefox_worker::Worker;
use crate::application::wait::with_timeout;
use crate::domain::emulation::{Geolocation, HttpCredentials};
use crate::domain::error::{Error, Result};
use crate::domain::storage::StorageState;

fn chromium_only(what: &str) -> Error {
    Error::InvalidArgument(format!(
        "{what} is only available in Chromium (Firefox speaks WebDriver BiDi, not CDP)"
    ))
}

fn bidi_headers(headers: &[(String, String)]) -> Value {
    json!(
        headers
            .iter()
            .map(|(k, v)| json!({"name": k, "value": {"type": "string", "value": v}}))
            .collect::<Vec<_>>()
    )
}

/// Firefox's `browser.browserType()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BrowserType;

impl BrowserType {
    /// `browserType.name()`.
    pub fn name(&self) -> &'static str {
        "firefox"
    }
}

impl Browser {
    /// `browser.browserType()`.
    pub fn browser_type(&self) -> BrowserType {
        BrowserType
    }

    /// `browser.contexts()`: the open contexts made through this browser.
    pub fn contexts(&self) -> Vec<BrowserContext> {
        let mut list = self.inner.contexts.lock().unwrap();
        list.retain(|c| {
            c.upgrade()
                .is_some_and(|c| !c.closed.load(Ordering::SeqCst))
        });
        list.iter()
            .filter_map(|c| c.upgrade())
            .map(BrowserContext::from_inner)
            .collect()
    }

    /// `browser.isConnected()`.
    pub fn is_connected(&self) -> bool {
        !self.inner.closed.load(Ordering::SeqCst)
    }

    /// `browser.newBrowserCDPSession()`: Chromium only.
    pub async fn new_browser_cdp_session(&self) -> Result<()> {
        Err(chromium_only("browser.newBrowserCDPSession()"))
    }

    /// `browser.startTracing()` (Chrome's performance trace): Chromium only.
    pub async fn start_tracing(&self, _page: Option<&Page>, _categories: &[&str]) -> Result<()> {
        Err(chromium_only("browser.startTracing()"))
    }

    /// `browser.stopTracing()`: Chromium only.
    pub async fn stop_tracing(&self) -> Result<Vec<u8>> {
        Err(chromium_only("browser.stopTracing()"))
    }
}

impl BrowserContext {
    /// `context.browser()`: the browser that made this context.
    pub fn browser(&self) -> Option<Browser> {
        self.inner.browser.upgrade().map(|inner| Browser { inner })
    }

    /// Whether `context.close()` ran (or the browser closed).
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
            || self
                .inner
                .browser
                .upgrade()
                .is_some_and(|b| b.closed.load(Ordering::SeqCst))
    }

    /// `context.backgroundPages()`: Chromium extension pages; none in Firefox.
    pub fn background_pages(&self) -> Vec<Page> {
        Vec::new()
    }

    /// `context.serviceWorkers()`: Playwright reports service workers in
    /// Chromium only; none in Firefox.
    pub fn service_workers(&self) -> Vec<Worker> {
        Vec::new()
    }

    /// `context.newCDPSession(page)`: Chromium only.
    pub async fn new_cdp_session(&self, _page: &Page) -> Result<()> {
        Err(chromium_only("context.newCDPSession()"))
    }

    /// `context.events()`: pages opened, their console messages, dialogs,
    /// requests and errors, and the context's close, from now on.
    pub fn events(&self) -> impl futures::Stream<Item = ContextEvent> + Send + use<> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        self.inner.subscribers.lock().unwrap().push(tx);
        rx
    }

    /// `context.waitForEvent(event)`: the first event `matcher` maps to a
    /// value. Listens from the call on, so start it before the action.
    pub fn wait_for_event<T, F>(
        &self,
        matcher: F,
    ) -> impl Future<Output = Result<T>> + Send + use<T, F>
    where
        F: Fn(&ContextEvent) -> Option<T> + Send + Sync + 'static,
        T: Send + 'static,
    {
        let mut events = self.events();
        let clock = self.inner.services.clock.clone();
        let timeout = self
            .inner
            .timeouts
            .lock()
            .unwrap()
            .action
            .unwrap_or(crate::application::firefox::DEFAULT_TIMEOUT);
        async move {
            with_timeout(&*clock, timeout, "waiting for context event", async {
                while let Some(e) = events.next().await {
                    if let Some(v) = matcher(&e) {
                        return Ok(v);
                    }
                    if matches!(e, ContextEvent::Close) {
                        break;
                    }
                }
                Err(Error::TargetClosed("context closed".into()))
            })
            .await
        }
    }

    /// `context.setExtraHTTPHeaders(headers)`: sent with every request of
    /// the context's pages (a page's own headers are added to these).
    pub async fn set_extra_http_headers(&self, headers: &[(&str, &str)]) -> Result<()> {
        *self.inner.extra_headers.lock().unwrap() = headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let merged = self.inner.extra_headers.lock().unwrap().clone();
        self.inner
            .bidi
            .call(
                "network.setExtraHeaders",
                json!({"headers": bidi_headers(&merged), "userContexts": [self.inner.user_context]}),
            )
            .await?;
        for page in self.live_pages() {
            page.apply_extra_headers().await?;
        }
        Ok(())
    }

    /// `context.setOffline(offline)`.
    pub async fn set_offline(&self, offline: bool) -> Result<()> {
        let conditions = if offline {
            json!({"type": "offline"})
        } else {
            Value::Null
        };
        self.inner
            .bidi
            .call(
                "emulation.setNetworkConditions",
                json!({"networkConditions": conditions, "userContexts": [self.inner.user_context]}),
            )
            .await
            .map(drop)
    }

    /// `context.setGeolocation(geolocation)` (`None` clears it).
    pub async fn set_geolocation(&self, geolocation: Option<Geolocation>) -> Result<()> {
        let coordinates = match geolocation {
            Some(g) => {
                json!({"latitude": g.latitude, "longitude": g.longitude, "accuracy": g.accuracy})
            }
            None => Value::Null,
        };
        self.inner
            .bidi
            .call(
                "emulation.setGeolocationOverride",
                json!({"coordinates": coordinates, "userContexts": [self.inner.user_context]}),
            )
            .await
            .map(drop)
    }

    /// `context.grantPermissions(permissions, { origin })`: for `origin`, or
    /// for every origin the context's pages are on or navigate to.
    pub async fn grant_permissions(
        &self,
        permissions: &[&str],
        origin: Option<&str>,
    ) -> Result<()> {
        let names: Vec<String> = permissions.iter().map(|p| (*p).to_owned()).collect();
        match origin.filter(|o| *o != "*") {
            Some(origin) => {
                let origin = crate::domain::firefox::origin_of(origin)
                    .unwrap_or_else(|| origin.trim_end_matches('/').to_owned());
                self.set_permissions_for(&origin, &names, "granted").await
            }
            None => {
                self.inner
                    .granted_any
                    .lock()
                    .unwrap()
                    .extend(names.iter().cloned());
                for page in self.live_pages() {
                    if let Some(origin) = crate::domain::firefox::origin_of(&page.url()) {
                        self.set_permissions_for(&origin, &names, "granted").await?;
                    }
                }
                Ok(())
            }
        }
    }

    /// `context.clearPermissions()`: every permission granted so far goes
    /// back to `prompt`.
    pub async fn clear_permissions(&self) -> Result<()> {
        self.inner.granted_any.lock().unwrap().clear();
        let granted: Vec<(String, String)> = self.inner.granted.lock().unwrap().drain(..).collect();
        for (origin, name) in granted {
            self.inner
                .bidi
                .call(
                    "permissions.setPermission",
                    json!({"descriptor": {"name": name}, "state": "prompt", "origin": origin,
                           "userContext": self.inner.user_context}),
                )
                .await?;
        }
        Ok(())
    }

    /// Sets `names` to `state` for `origin` in this user context.
    pub(crate) async fn set_permissions_for(
        &self,
        origin: &str,
        names: &[String],
        state: &str,
    ) -> Result<()> {
        for name in names {
            self.inner
                .bidi
                .call(
                    "permissions.setPermission",
                    json!({"descriptor": {"name": name}, "state": state, "origin": origin,
                           "userContext": self.inner.user_context}),
                )
                .await?;
            let mut granted = self.inner.granted.lock().unwrap();
            let pair = (origin.to_owned(), name.clone());
            if !granted.contains(&pair) {
                granted.push(pair);
            }
        }
        Ok(())
    }

    /// `context.setHTTPCredentials(credentials)` (`None` stops answering
    /// HTTP authentication).
    pub async fn set_http_credentials(&self, credentials: Option<HttpCredentials>) -> Result<()> {
        let had = self.inner.credentials.lock().unwrap().is_some();
        let has = credentials.is_some();
        *self.inner.credentials.lock().unwrap() = credentials;
        if has && !had {
            for page in self.live_pages() {
                page.bidi_call(
                    "network.addIntercept",
                    json!({"phases": ["authRequired"], "contexts": [page.main_context()]}),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// `context.setStorageState(state)`: replaces the cookies, and the
    /// localStorage of the state's origins and the pages' origins.
    pub async fn set_storage_state(&self, state: &StorageState) -> Result<()> {
        self.clear_cookies().await?;
        for page in self.live_pages() {
            let _ = page
                .evaluate::<Value>("(() => { try { localStorage.clear(); } catch {} })()")
                .await;
        }
        self.restore_storage_state(state).await
    }
}

impl Page {
    /// `page.setExtraHTTPHeaders(headers)`: added to the context's for this
    /// page's requests.
    pub async fn set_extra_http_headers(&self, headers: &[(&str, &str)]) -> Result<()> {
        *self.inner.extra_headers.lock().unwrap() = headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        self.apply_extra_headers().await
    }

    /// Sets this page's headers: the context's, then its own (BiDi's
    /// per-context headers replace the user context's).
    pub(crate) async fn apply_extra_headers(&self) -> Result<()> {
        let own = self.inner.extra_headers.lock().unwrap().clone();
        if own.is_empty() {
            return Ok(());
        }
        let mut merged: Vec<(String, String)> = self
            .inner
            .owner
            .upgrade()
            .map(|c| c.extra_headers.lock().unwrap().clone())
            .unwrap_or_default();
        merged.retain(|(k, _)| !own.iter().any(|(o, _)| o.eq_ignore_ascii_case(k)));
        merged.extend(own);
        self.bidi_call(
            "network.setExtraHeaders",
            json!({"headers": bidi_headers(&merged), "contexts": [self.main_context()]}),
        )
        .await
        .map(drop)
    }
}
