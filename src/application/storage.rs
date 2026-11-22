//! Cookies, storage state, permissions and runtime context settings
//! (`context.cookies()`, `storageState()`, `setOffline()`, ...).

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::application::context::{BrowserContext, ContextInner};
use crate::application::page::{DEFAULT_TIMEOUT, Page};
use crate::application::route::Route;
use crate::domain::emulation::Geolocation;
use crate::domain::error::{Error, Result};
use crate::domain::frame::World;
use crate::domain::storage::{
    ClearCookies, Cookie, NameValue, NewCookie, OriginState, StorageState, filter_cookies,
    protocol_permissions,
};

/// Context settings changed after creation; they override the options for
/// open pages and for pages opened later.
#[derive(Default)]
pub(crate) struct RuntimeSettings {
    pub(crate) extra_http_headers: Option<BTreeMap<String, String>>,
    pub(crate) offline: Option<bool>,
    pub(crate) geolocation: Option<Option<Geolocation>>,
    pub(crate) http_credentials: Option<Option<crate::domain::emulation::HttpCredentials>>,
}

pub(crate) type Origins = Mutex<Vec<String>>;

/// Playwright's storage script, as a `StorageScript` instance.
fn storage_script() -> String {
    format!(
        "(() => {{ const module = {{ exports: {{}} }};\n{}\nreturn new (module.exports.StorageScript())('chromium'); }})()",
        include_str!("scripts/storage.js")
    )
}

/// Playwright's `StorageStateOptions` for `storageState()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StorageStateOptions {
    /// Also serialise IndexedDB databases.
    pub indexed_db: bool,
    /// Also serialise the origin private file system (Playwright 1.64's `opfs`).
    pub opfs: bool,
}

/// What the storage script collects for one origin.
pub(crate) struct Collected {
    local_storage: Vec<NameValue>,
    indexed_db: Option<Vec<Value>>,
    opfs: Option<Vec<Value>>,
}

/// The `http(s)` origin of `url`.
pub(crate) fn origin_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    Some(parsed.origin().ascii_serialization())
}

fn offline_params(offline: bool) -> Value {
    json!({"offline": offline, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1})
}

fn geolocation_params(g: Option<Geolocation>) -> Value {
    match g {
        Some(g) => {
            json!({"latitude": g.latitude, "longitude": g.longitude, "accuracy": g.accuracy})
        }
        None => json!({}),
    }
}

/// Context headers, then page headers on top (names compared case-insensitively).
fn merge_headers(
    context: &BTreeMap<String, String>,
    page: Option<&BTreeMap<String, String>>,
) -> BTreeMap<String, String> {
    let mut out = context.clone();
    for (k, v) in page.into_iter().flatten() {
        out.retain(|name, _| !name.eq_ignore_ascii_case(k));
        out.insert(k.clone(), v.clone());
    }
    out
}

impl ContextInner {
    pub(crate) fn add_origin(&self, origin: String) {
        let mut origins = self.origins.lock().unwrap();
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }

    /// The credentials in effect (`setHTTPCredentials` overrides the option).
    pub(crate) fn http_credentials(&self) -> Option<crate::domain::emulation::HttpCredentials> {
        match &self.runtime.lock().unwrap().http_credentials {
            Some(c) => c.clone(),
            None => self.options.http_credentials.clone(),
        }
    }

    fn context_headers(&self) -> BTreeMap<String, String> {
        self.runtime
            .lock()
            .unwrap()
            .extra_http_headers
            .clone()
            .unwrap_or_else(|| self.options.extra_http_headers.clone())
    }
}

/// Commands that apply runtime settings to a new page of `context`.
pub(crate) fn runtime_commands(context: &ContextInner) -> Vec<(&'static str, Value)> {
    let rt = context.runtime.lock().unwrap();
    let mut out = Vec::new();
    if let Some(h) = &rt.extra_http_headers {
        out.push(("Network.setExtraHTTPHeaders", json!({"headers": h})));
    }
    if let Some(offline) = rt.offline {
        out.push(("Network.emulateNetworkConditions", offline_params(offline)));
    }
    if let Some(g) = rt.geolocation {
        out.push(("Emulation.setGeolocationOverride", geolocation_params(g)));
    }
    out
}

impl Page {
    pub(crate) fn http_credentials(&self) -> Option<crate::domain::emulation::HttpCredentials> {
        match self.inner.context.upgrade() {
            Some(c) => c.http_credentials(),
            None => self.inner.options.http_credentials.clone(),
        }
    }

    /// `page.setExtraHTTPHeaders(headers)`: sent with every request of this
    /// page, on top of the context's.
    pub async fn set_extra_http_headers<K: Into<String>, V: Into<String>>(
        &self,
        headers: impl IntoIterator<Item = (K, V)>,
    ) -> Result<()> {
        let headers: BTreeMap<String, String> = headers
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        *self.inner.extra_http_headers.lock().unwrap() = Some(headers);
        self.update_extra_http_headers().await
    }

    pub(crate) async fn update_extra_http_headers(&self) -> Result<()> {
        let context = self
            .inner
            .context
            .upgrade()
            .map(|c| c.context_headers())
            .unwrap_or_else(|| self.inner.options.extra_http_headers.clone());
        let page = self.inner.extra_http_headers.lock().unwrap().clone();
        let merged = merge_headers(&context, page.as_ref());
        self.send("Network.setExtraHTTPHeaders", json!({"headers": merged}))
            .await?;
        Ok(())
    }

    /// localStorage (and IndexedDB / OPFS when asked) of the current origin.
    async fn collect_storage(&self, options: StorageStateOptions) -> Result<Collected> {
        let v = self
            .call_function(
                World::Utility,
                &format!(
                    "async function(record) {{ return await {}.collect(record); }}",
                    storage_script()
                ),
                &[json!({"indexedDB": options.indexed_db, "opfs": options.opfs})],
                DEFAULT_TIMEOUT,
                "collecting storage",
            )
            .await?;
        Ok(Collected {
            local_storage: serde_json::from_value(v["localStorage"].clone())?,
            indexed_db: v.get("indexedDB").and_then(Value::as_array).cloned(),
            opfs: v.get("opfs").and_then(Value::as_array).cloned(),
        })
    }
}

impl BrowserContext {
    async fn browser_call(&self, method: &str, mut params: Value) -> Result<Value> {
        if !self.inner.id.is_empty() {
            params["browserContextId"] = json!(self.inner.id);
        }
        self.inner.shared.cdp.call(None, method, params).await
    }

    /// `context.cookies()`: every cookie of the context.
    pub async fn cookies(&self) -> Result<Vec<Cookie>> {
        self.cookies_for(&[]).await
    }

    /// `context.cookies(urls)`: the cookies sent to any of `urls`.
    pub async fn cookies_for(&self, urls: &[&str]) -> Result<Vec<Cookie>> {
        let r = self.browser_call("Storage.getCookies", json!({})).await?;
        let cookies = r
            .get("cookies")
            .and_then(Value::as_array)
            .map(|list| list.iter().map(Cookie::from_protocol).collect())
            .unwrap_or_default();
        Ok(filter_cookies(cookies, urls))
    }

    /// `context.addCookies(cookies)`.
    pub async fn add_cookies(&self, cookies: &[NewCookie]) -> Result<()> {
        let list = cookies
            .iter()
            .map(|c| c.resolve().map(|c| c.to_protocol()))
            .collect::<std::result::Result<Vec<Value>, String>>()
            .map_err(Error::InvalidArgument)?;
        self.browser_call("Storage.setCookies", json!({"cookies": list}))
            .await?;
        Ok(())
    }

    /// `context.clearCookies()`.
    pub async fn clear_cookies(&self) -> Result<()> {
        self.browser_call("Storage.clearCookies", json!({})).await?;
        Ok(())
    }

    /// `context.clearCookies({name, domain, path})`: only matching cookies.
    pub async fn clear_cookies_matching(&self, filter: ClearCookies) -> Result<()> {
        let keep: Vec<NewCookie> = self
            .cookies()
            .await?
            .iter()
            .filter(|c| !filter.matches(c))
            .map(Cookie::to_new_cookie)
            .collect();
        self.clear_cookies().await?;
        if !keep.is_empty() {
            self.add_cookies(&keep).await?;
        }
        Ok(())
    }

    /// `context.grantPermissions(permissions, {origin})`.
    pub async fn grant_permissions(
        &self,
        permissions: &[&str],
        origin: Option<&str>,
    ) -> Result<()> {
        let mapped = protocol_permissions(permissions).map_err(Error::InvalidArgument)?;
        let mut params = json!({"permissions": mapped});
        if let Some(o) = origin.filter(|o| *o != "*") {
            params["origin"] = json!(o);
        }
        match self
            .browser_call("Browser.grantPermissions", params.clone())
            .await
        {
            Err(Error::Protocol { .. }) if permissions.contains(&"local-network-access") => {
                // Older Chromium only knows `localNetworkAccess`.
                let fallback: Vec<&str> = mapped
                    .into_iter()
                    .filter(|p| !matches!(*p, "localNetwork" | "loopbackNetwork"))
                    .collect();
                params["permissions"] = json!(fallback);
                self.browser_call("Browser.grantPermissions", params)
                    .await?;
                Ok(())
            }
            other => other.map(|_| ()),
        }
    }

    /// `context.clearPermissions()`.
    pub async fn clear_permissions(&self) -> Result<()> {
        self.browser_call("Browser.resetPermissions", json!({}))
            .await?;
        Ok(())
    }

    fn live_pages(&self) -> Vec<Page> {
        self.pages()
            .into_iter()
            .filter(|p| !p.is_closed())
            .collect()
    }

    /// `context.setExtraHTTPHeaders(headers)`.
    pub async fn set_extra_http_headers<K: Into<String>, V: Into<String>>(
        &self,
        headers: impl IntoIterator<Item = (K, V)>,
    ) -> Result<()> {
        let headers: BTreeMap<String, String> = headers
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        self.inner.runtime.lock().unwrap().extra_http_headers = Some(headers);
        for page in self.live_pages() {
            page.update_extra_http_headers().await?;
        }
        Ok(())
    }

    /// `context.setHTTPCredentials(credentials)` (deprecated in Playwright,
    /// still supported): `None` stops answering auth challenges.
    pub async fn set_http_credentials(
        &self,
        credentials: Option<crate::domain::emulation::HttpCredentials>,
    ) -> Result<()> {
        self.inner.runtime.lock().unwrap().http_credentials = Some(credentials);
        for page in self.live_pages() {
            page.update_interception().await?;
        }
        Ok(())
    }

    /// `context.setOffline(offline)`.
    pub async fn set_offline(&self, offline: bool) -> Result<()> {
        self.inner.runtime.lock().unwrap().offline = Some(offline);
        for page in self.live_pages() {
            page.send("Network.emulateNetworkConditions", offline_params(offline))
                .await?;
        }
        Ok(())
    }

    /// `context.setGeolocation(geolocation)`; `None` clears the override.
    pub async fn set_geolocation(&self, geolocation: Option<Geolocation>) -> Result<()> {
        if geolocation.is_some() {
            crate::domain::emulation::ContextOptions {
                geolocation,
                ..Default::default()
            }
            .validate()?;
        }
        self.inner.runtime.lock().unwrap().geolocation = Some(geolocation);
        for page in self.live_pages() {
            page.send(
                "Emulation.setGeolocationOverride",
                geolocation_params(geolocation),
            )
            .await?;
        }
        Ok(())
    }

    /// `context.storageState()`: cookies plus `localStorage` of every origin
    /// the context has visited.
    pub async fn storage_state(&self) -> Result<StorageState> {
        self.storage_state_with(StorageStateOptions::default())
            .await
    }

    /// `context.storageState({ indexedDB })`.
    pub async fn storage_state_with(&self, options: StorageStateOptions) -> Result<StorageState> {
        let cookies = self.cookies().await?;
        let mut pending: Vec<String> = self.inner.origins.lock().unwrap().clone();
        let mut origins = Vec::new();
        let keep = |origin: String, c: Collected| {
            let some = |v: &Option<Vec<Value>>| v.as_ref().is_some_and(|d| !d.is_empty());
            (!c.local_storage.is_empty() || some(&c.indexed_db) || some(&c.opfs)).then_some(
                OriginState {
                    origin,
                    local_storage: c.local_storage,
                    indexed_db: c.indexed_db.filter(|d| !d.is_empty()),
                    opfs: c.opfs.filter(|d| !d.is_empty()),
                },
            )
        };
        for page in self.live_pages() {
            let Some(origin) = origin_of(&page.url()) else {
                continue;
            };
            if !pending.contains(&origin) {
                continue;
            }
            if let Ok(collected) = page.collect_storage(options).await {
                pending.retain(|o| *o != origin);
                origins.extend(keep(origin, collected));
            }
        }
        if !pending.is_empty() {
            self.with_storage_page(&pending, |page, origin| {
                let origin = origin.to_owned();
                async move { Ok(keep(origin, page.collect_storage(options).await?)) }
            })
            .await?
            .into_iter()
            .flatten()
            .for_each(|o| origins.push(o));
        }
        Ok(StorageState { cookies, origins })
    }

    /// `context.setStorageState(state)`: replaces cookies and the storage of
    /// every visited origin with `state`.
    pub async fn set_storage_state(&self, state: &StorageState) -> Result<()> {
        self.clear_cookies().await?;
        let visited = self.inner.origins.lock().unwrap().clone();
        let mut full = state.clone();
        for origin in visited {
            if !full.origins.iter().any(|o| o.origin == origin) {
                full.origins.push(OriginState {
                    origin,
                    local_storage: Vec::new(),
                    indexed_db: None,
                    opfs: None,
                });
            }
        }
        self.restore_storage_state(&full).await?;
        *self.inner.origins.lock().unwrap() =
            state.origins.iter().map(|o| o.origin.clone()).collect();
        Ok(())
    }

    /// Applies a storage state to this (new) context.
    pub(crate) async fn restore_storage_state(&self, state: &StorageState) -> Result<()> {
        if !state.cookies.is_empty() {
            let cookies: Vec<NewCookie> = state.cookies.iter().map(Cookie::to_new_cookie).collect();
            self.add_cookies(&cookies).await?;
        }
        let names: Vec<String> = state.origins.iter().map(|o| o.origin.clone()).collect();
        if !names.is_empty() {
            self.with_storage_page(&names, |page, origin| {
                let origin_state = state.origins.iter().find(|o| o.origin == origin).cloned();
                async move {
                    page.call_function(
                        World::Utility,
                        &format!(
                            "async function(state) {{ await {}.restore(state); }}",
                            storage_script()
                        ),
                        &[serde_json::to_value(origin_state)?],
                        DEFAULT_TIMEOUT,
                        "restoring storage",
                    )
                    .await?;
                    Ok(())
                }
            })
            .await
            .map_err(|e| Error::InvalidArgument(format!("Error setting storage state:\n{e}")))?;
        }
        *self.inner.origins.lock().unwrap() = names;
        Ok(())
    }

    /// Opens a helper page whose requests all get an empty document, visits
    /// each origin and runs `f` there, then closes the page.
    async fn with_storage_page<T, F, Fut>(&self, origins: &[String], f: F) -> Result<Vec<T>>
    where
        F: Fn(Page, &str) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let page = self.new_page().await?;
        let run = async {
            page.route("**/*", |r: Route| async move {
                let _ = r.fulfill().body("<html></html>").await;
            })
            .await?;
            let mut out = Vec::new();
            for origin in origins {
                page.goto(origin).await?;
                out.push(f(page.clone(), origin).await?);
            }
            Ok(out)
        };
        let result = run.await;
        let _ = page.close().await;
        result
    }
}

/// `page.localStorage` / `page.sessionStorage` of the main frame's origin.
#[derive(Debug, Clone)]
pub struct WebStorage {
    page: Page,
    name: &'static str,
}

impl WebStorage {
    async fn call(&self, function: &str, args: &[Value]) -> Result<Value> {
        let declaration = format!(
            "(name, ...args) => {{ const s = globalThis[name]; return ({function})(s, ...args); }}"
        );
        let mut all = vec![json!(self.name)];
        all.extend_from_slice(args);
        self.page
            .call_function(
                World::Utility,
                &declaration,
                &all,
                self.page.default_timeout(),
                "using web storage",
            )
            .await
    }

    /// All entries.
    pub async fn items(&self) -> Result<Vec<NameValue>> {
        let v = self
            .call(
                "s => Object.keys(s).map(name => ({name, value: s.getItem(name)}))",
                &[],
            )
            .await?;
        Ok(serde_json::from_value(v)?)
    }

    pub async fn get_item(&self, name: &str) -> Result<Option<String>> {
        let v = self.call("(s, k) => s.getItem(k)", &[json!(name)]).await?;
        Ok(v.as_str().map(str::to_owned))
    }

    pub async fn set_item(&self, name: &str, value: &str) -> Result<()> {
        self.call(
            "(s, k, v) => { s.setItem(k, v); }",
            &[json!(name), json!(value)],
        )
        .await?;
        Ok(())
    }

    pub async fn remove_item(&self, name: &str) -> Result<()> {
        self.call("(s, k) => { s.removeItem(k); }", &[json!(name)])
            .await?;
        Ok(())
    }

    pub async fn clear(&self) -> Result<()> {
        self.call("s => { s.clear(); }", &[]).await?;
        Ok(())
    }
}

impl Page {
    /// `page.localStorage`.
    pub fn local_storage(&self) -> WebStorage {
        WebStorage {
            page: self.clone(),
            name: "localStorage",
        }
    }

    /// `page.sessionStorage`.
    pub fn session_storage(&self) -> WebStorage {
        WebStorage {
            page: self.clone(),
            name: "sessionStorage",
        }
    }
}
