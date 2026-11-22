//! Playwright's `$`, `$$`, `$eval` and `$$eval` on pages, frames and
//! element handles: immediate queries (no waiting). `$` and `$eval` use the
//! first match unless the context has `strictSelectors`.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::application::frame::Frame;
use crate::application::handle::JsHandle;
use crate::application::locator::Locator;
use crate::application::page::Page;
use crate::domain::error::Result;

/// Something `$`-style queries start from.
trait QueryRoot {
    async fn query_locator(&self, selector: &str) -> Result<Locator>;
}

impl QueryRoot for Page {
    async fn query_locator(&self, selector: &str) -> Result<Locator> {
        Ok(Locator::for_selector(self, None, selector))
    }
}

impl QueryRoot for Frame {
    async fn query_locator(&self, selector: &str) -> Result<Locator> {
        Ok(Locator::for_selector(
            self.page(),
            Some(self.id().to_owned()),
            selector,
        ))
    }
}

impl QueryRoot for JsHandle {
    /// Inside the element (Playwright's `elementHandle.$`).
    async fn query_locator(&self, selector: &str) -> Result<Locator> {
        Ok(self.as_locator().await?.locator_for_selector(selector))
    }
}

macro_rules! query_api {
    ($ty:ty, $doc:literal) => {
        impl $ty {
            #[doc = concat!("`", $doc, ".$(selector)`: the first matching element, if any.")]
            pub async fn query_selector(&self, selector: &str) -> Result<Option<JsHandle>> {
                self.query_locator(selector).await?.query_handle().await
            }

            #[doc = concat!("`", $doc, ".$$(selector)`: every matching element.")]
            pub async fn query_selector_all(&self, selector: &str) -> Result<Vec<JsHandle>> {
                self.query_locator(selector).await?.element_handles().await
            }

            #[doc = concat!("`", $doc, ".$eval(selector, fn)`: `fn(element)` on the first match.")]
            pub async fn eval_on_selector<T: DeserializeOwned>(
                &self,
                selector: &str,
                function: &str,
            ) -> Result<T> {
                self.query_locator(selector)
                    .await?
                    .eval_on_first::<T, ()>(function, None)
                    .await
            }

            #[doc = concat!("`", $doc, ".$eval(selector, fn, arg)`.")]
            pub async fn eval_on_selector_with<T: DeserializeOwned, A: Serialize>(
                &self,
                selector: &str,
                function: &str,
                arg: A,
            ) -> Result<T> {
                self.query_locator(selector)
                    .await?
                    .eval_on_first(function, Some(arg))
                    .await
            }

            #[doc = concat!("`", $doc, ".$$eval(selector, fn)`: `fn(elements)`.")]
            pub async fn eval_on_selector_all<T: DeserializeOwned>(
                &self,
                selector: &str,
                function: &str,
            ) -> Result<T> {
                self.query_locator(selector)
                    .await?
                    .evaluate_all(function)
                    .await
            }

            #[doc = concat!("`", $doc, ".$$eval(selector, fn, arg)`.")]
            pub async fn eval_on_selector_all_with<T: DeserializeOwned, A: Serialize>(
                &self,
                selector: &str,
                function: &str,
                arg: A,
            ) -> Result<T> {
                self.query_locator(selector)
                    .await?
                    .evaluate_all_with(function, arg)
                    .await
            }
        }
    };
}

query_api!(Page, "page");
query_api!(Frame, "frame");
query_api!(JsHandle, "elementHandle");
