//! A browser driven through the extension's page protocol (Firefox and
//! Safari, whose extensions get no debugger): tabs, `goto`, `evaluate`,
//! click, fill and screenshots.

use std::sync::Arc;
use std::sync::Mutex;

use base64::Engine;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::domain::error::{Error, Result};
use crate::ports::CdpSession;

/// A browser driven through the extension's page protocol.
#[derive(Clone)]
pub struct PageBrowser {
    session: Arc<dyn CdpSession>,
    user_agent: String,
}

/// One of its tabs.
#[derive(Clone)]
pub struct PageTab {
    session: Arc<dyn CdpSession>,
    id: i64,
    info: Arc<Mutex<(String, String)>>,
}

impl std::fmt::Debug for PageTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageTab")
            .field("id", &self.id)
            .field("url", &self.url())
            .finish()
    }
}

impl PageBrowser {
    pub(crate) fn new(session: Arc<dyn CdpSession>, user_agent: String) -> Self {
        Self {
            session,
            user_agent,
        }
    }

    /// The browser's user agent, as the extension reported it.
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// The tabs the extension can script.
    pub async fn tabs(&self) -> Result<Vec<PageTab>> {
        let r = self.session.call(None, "Tabs.list", json!({})).await?;
        Ok(r["tabs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| self.tab(t["tabId"].as_i64().unwrap_or_default(), t))
            .collect())
    }

    /// Opens a tab at `url`, once loaded.
    pub async fn new_tab(&self, url: &str) -> Result<PageTab> {
        let r = self
            .session
            .call(None, "Tabs.create", json!({"url": url}))
            .await?;
        let id = r["tabId"]
            .as_i64()
            .ok_or_else(|| Error::Serde("Tabs.create returned no tabId".into()))?;
        let tab = self.tab(id, &json!({}));
        tab.refresh().await?;
        Ok(tab)
    }

    /// Disconnects; the browser and its tabs stay.
    pub async fn close(&self) {
        self.session.disconnect();
    }

    fn tab(&self, id: i64, info: &Value) -> PageTab {
        let text = |k: &str| info[k].as_str().unwrap_or_default().to_owned();
        PageTab {
            session: self.session.clone(),
            id,
            info: Arc::new(Mutex::new((text("url"), text("title")))),
        }
    }
}

impl PageTab {
    /// The browser's id for the tab.
    pub fn id(&self) -> i64 {
        self.id
    }

    /// The URL, as of the last navigation or [`refresh`](Self::refresh).
    pub fn url(&self) -> String {
        self.info.lock().unwrap().0.clone()
    }

    /// The title, as of the last navigation or [`refresh`](Self::refresh).
    pub fn title(&self) -> String {
        self.info.lock().unwrap().1.clone()
    }

    async fn call(&self, method: &str, mut params: Value) -> Result<Value> {
        params["tabId"] = json!(self.id);
        self.session.call(None, method, params).await
    }

    fn remember(&self, r: &Value) {
        let mut info = self.info.lock().unwrap();
        if let Some(url) = r["url"].as_str() {
            info.0 = url.to_owned();
        }
        if let Some(title) = r["title"].as_str() {
            info.1 = title.to_owned();
        }
    }

    /// Reads the tab's current URL and title.
    pub async fn refresh(&self) -> Result<()> {
        let r = self.call("Tab.info", json!({})).await?;
        self.remember(&r);
        Ok(())
    }

    /// Loads `url` and waits for it to load.
    pub async fn goto(&self, url: &str) -> Result<()> {
        let r = self.call("Tab.goto", json!({"url": url})).await?;
        self.remember(&r);
        Ok(())
    }

    /// Runs `expression` in the page (promises are awaited); its JSON value.
    pub async fn evaluate<T: DeserializeOwned>(&self, expression: &str) -> Result<T> {
        let r = self
            .call("Tab.evaluate", json!({"expression": expression}))
            .await?;
        Ok(serde_json::from_value(r["value"].clone())?)
    }

    /// Clicks the first element matching the CSS `selector`.
    pub async fn click(&self, selector: &str) -> Result<()> {
        self.call("Tab.click", json!({"selector": selector}))
            .await
            .map(drop)
    }

    /// Sets the value of the first input matching `selector` (with `input`
    /// and `change` events).
    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> {
        self.call("Tab.fill", json!({"selector": selector, "value": value}))
            .await
            .map(drop)
    }

    /// The visible tab as a PNG (the tab is brought to the front).
    pub async fn screenshot(&self) -> Result<Vec<u8>> {
        let r = self.call("Tab.screenshot", json!({})).await?;
        base64::engine::general_purpose::STANDARD
            .decode(r["data"].as_str().unwrap_or_default())
            .map_err(|e| Error::Serde(e.to_string()))
    }

    /// Closes the tab.
    pub async fn close(&self) -> Result<()> {
        self.call("Tab.close", json!({})).await.map(drop)
    }
}
