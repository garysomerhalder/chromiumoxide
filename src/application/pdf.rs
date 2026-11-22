//! `page.pdf()`.

use std::future::IntoFuture;
use std::path::PathBuf;

use base64::Engine;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::pdf::PdfOptions;

/// A pending `page.pdf()`: resolves to the PDF bytes.
#[must_use = "does nothing until awaited"]
pub struct Pdf<'a> {
    page: &'a Page,
    options: PdfOptions,
    path: Option<PathBuf>,
}

impl<'a> Pdf<'a> {
    pub fn options(mut self, options: PdfOptions) -> Self {
        self.options = options;
        self
    }

    /// Also writes the PDF to this file.
    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }
}

impl Page {
    /// `page.pdf()` with Playwright's defaults (Letter, no margins).
    pub fn pdf(&self) -> Pdf<'_> {
        Pdf {
            page: self,
            options: PdfOptions::default(),
            path: None,
        }
    }

    /// Reads a protocol stream (`IO.read`) to the end.
    pub(crate) async fn read_stream(&self, handle: &str) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let chunk = self.send("IO.read", json!({"handle": handle})).await?;
            let data = chunk
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if chunk.get("base64Encoded").and_then(Value::as_bool) == Some(true) {
                out.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .map_err(|e| Error::Serde(format!("invalid stream chunk: {e}")))?,
                );
            } else {
                out.extend_from_slice(data.as_bytes());
            }
            if chunk.get("eof").and_then(Value::as_bool) == Some(true) {
                break;
            }
        }
        let _ = self.send("IO.close", json!({"handle": handle})).await;
        Ok(out)
    }
}

impl<'a> IntoFuture for Pdf<'a> {
    type Output = Result<Vec<u8>>;
    type IntoFuture = BoxFuture<'a, Result<Vec<u8>>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let params = self.options.to_protocol().map_err(Error::InvalidArgument)?;
            let r = self.page.send("Page.printToPDF", params).await?;
            let handle = r
                .get("stream")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Serde("printToPDF returned no stream".into()))?;
            let pdf = self.page.read_stream(handle).await?;
            if let Some(path) = &self.path {
                self.page.inner.shared.services.fs.write(path, &pdf)?;
            }
            Ok(pdf)
        }
        .boxed()
    }
}
