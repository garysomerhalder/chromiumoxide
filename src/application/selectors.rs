//! `selectors`: the test id attribute and custom selector engines, per browser.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::application::browser::Browser;
use crate::application::frame::Frame;
use crate::application::locator::{FrameLocator, HANDLE_ENGINE, Locator};
use crate::application::page::Page;
use crate::application::shared::Shared;
use crate::domain::error::{Error, Result};

/// Selector settings shared by a browser's pages.
#[derive(Debug, Clone)]
pub(crate) struct SelectorConfig {
    test_id_attribute: String,
    /// `(name, source, content_script)`.
    engines: Vec<(String, String, bool)>,
}

/// Playwright's `selectors.register(name, script, options)` options.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectorRegisterOptions {
    /// Run the engine isolated from the page's scripts (Playwright's
    /// `contentScript`); by default it runs in the page's main world.
    pub content_script: bool,
}

impl Default for SelectorConfig {
    fn default() -> Self {
        SelectorConfig {
            test_id_attribute: "data-testid".into(),
            engines: Vec::new(),
        }
    }
}

/// Playwright's `selectors` (for this browser). Settings apply to pages
/// whose scripts start afterwards, as in Playwright.
#[derive(Clone)]
pub struct Selectors {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Selectors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Selectors").finish()
    }
}

impl Selectors {
    /// `selectors.setTestIdAttribute(name)`: used by `get_by_test_id`.
    pub fn set_test_id_attribute(&self, name: &str) {
        self.shared.selectors.lock().unwrap().test_id_attribute = name.to_owned();
    }

    /// `selectors.register(name, script)`: `script` evaluates to an engine
    /// object with `query(root, selector)` and `queryAll(root, selector)`.
    pub fn register(&self, name: &str, script: &str) -> Result<()> {
        self.register_with(name, script, SelectorRegisterOptions::default())
    }

    /// `selectors.register(name, script, { contentScript })`.
    pub fn register_with(
        &self,
        name: &str,
        script: &str,
        options: SelectorRegisterOptions,
    ) -> Result<()> {
        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return Err(Error::InvalidArgument(
                "Selector engine name may only contain [a-zA-Z0-9_] characters".into(),
            ));
        }
        let mut cfg = self.shared.selectors.lock().unwrap();
        if cfg.engines.iter().any(|(n, ..)| n == name) {
            return Err(Error::InvalidArgument(format!(
                "\"{name}\" selector engine has been already registered"
            )));
        }
        cfg.engines
            .push((name.to_owned(), script.to_owned(), options.content_script));
        Ok(())
    }
}

impl Browser {
    /// `playwright.selectors`, for this browser's pages.
    pub fn selectors(&self) -> Selectors {
        Selectors {
            shared: self.shared_ref().expect("selectors() on a closed browser"),
        }
    }
}

impl Page {
    pub(crate) fn page_ref(&self) -> &Page {
        self
    }

    pub(crate) fn test_id_attribute(&self) -> String {
        self.inner
            .shared
            .selectors
            .lock()
            .unwrap()
            .test_id_attribute
            .clone()
    }

    /// Playwright's `parseSelector` world rule: a custom engine that is not
    /// a content script runs in the main world. Detected by its `name=`
    /// prefix, so a match inside quoted text only costs the isolation.
    pub(crate) fn uses_main_world_engine(&self, selector: &str) -> bool {
        let cfg = self.inner.shared.selectors.lock().unwrap();
        cfg.engines
            .iter()
            .filter(|(_, _, content_script)| !content_script)
            .any(|(name, ..)| selector.contains(&format!("{name}=")))
    }

    pub(crate) fn custom_engines(&self) -> Value {
        let cfg = self.inner.shared.selectors.lock().unwrap();
        let mut engines = vec![json!({"name": "chromiumoxide-handle", "source": HANDLE_ENGINE})];
        engines.extend(
            cfg.engines
                .iter()
                .map(|(name, source, _)| json!({"name": name, "source": source})),
        );
        Value::Array(engines)
    }
}

impl Locator {
    pub(crate) fn page_ref(&self) -> &Page {
        self.page()
    }
}

impl Frame {
    pub(crate) fn page_ref(&self) -> &Page {
        self.page()
    }
}

impl FrameLocator {
    pub(crate) fn page_ref(&self) -> &Page {
        self.page_of()
    }
}
