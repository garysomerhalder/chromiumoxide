//! Use case: page screenshots, following Playwright's `Screenshotter`.

use std::future::IntoFuture;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::frame::World;
use crate::domain::screenshot::{
    Animations, Caret, ImageFormat, Rect, ScreenshotOptions, ScreenshotScale, Size, VisualViewport,
    capture, plan_page, validate,
};

const PREPARE: &str = include_str!("scripts/prepare_screenshot.js");

const CLEANUP: &str =
    "function() { window.__pwCleanupScreenshot && window.__pwCleanupScreenshot(); }";

const VIEWPORT_SIZE: &str =
    "function() { return { width: window.innerWidth, height: window.innerHeight }; }";

/// Playwright's `_fullPageSize`.
const FULL_PAGE_SIZE: &str = "function() {
  if (!document.body || !document.documentElement) return null;
  return {
    width: Math.max(document.body.scrollWidth, document.documentElement.scrollWidth, document.body.offsetWidth,
      document.documentElement.offsetWidth, document.body.clientWidth, document.documentElement.clientWidth),
    height: Math.max(document.body.scrollHeight, document.documentElement.scrollHeight, document.body.offsetHeight,
      document.documentElement.offsetHeight, document.body.clientHeight, document.documentElement.clientHeight),
  };
}";

impl Page {
    /// Playwright's `page.screenshot()`; chain options, then `.await` the PNG/JPEG bytes.
    pub fn screenshot(&self) -> Screenshot<'_> {
        Screenshot {
            page: self,
            options: ScreenshotOptions::default(),
            path: None,
            timeout: None,
        }
    }

    async fn run_screenshot(&self, o: &ScreenshotOptions, timeout: Duration) -> Result<Vec<u8>> {
        let quality = validate(o)?;
        let viewport = match self.viewport_size() {
            Some(v) => Size {
                width: v.width as f64,
                height: v.height as f64,
            },
            None => {
                let v = self
                    .call_function(
                        World::Utility,
                        VIEWPORT_SIZE,
                        &[],
                        timeout,
                        "reading the viewport",
                    )
                    .await?;
                Size {
                    width: num(&v, "width"),
                    height: num(&v, "height"),
                }
            }
        };
        let prepare = format!(
            "function(style, hideCaret, disableAnimations, syncAnimations) {{ return (\n{PREPARE}\n)(style, hideCaret, disableAnimations, syncAnimations); }}"
        );
        let args = [
            o.style.clone().map(Value::String).unwrap_or(Value::Null),
            json!(o.caret != Caret::Initial),
            json!(o.animations == Animations::Disabled),
            json!(false),
        ];
        self.call_function(
            World::Utility,
            &prepare,
            &args,
            timeout,
            "preparing the screenshot",
        )
        .await?;
        let result = self.capture_prepared(o, quality, viewport, timeout).await;
        let _ = self
            .call_function(
                World::Utility,
                CLEANUP,
                &[],
                timeout,
                "restoring after the screenshot",
            )
            .await;
        result
    }

    async fn capture_prepared(
        &self,
        o: &ScreenshotOptions,
        quality: Option<u8>,
        viewport: Size,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        // Playwright waits for fonts but ignores failures.
        let _ = self
            .call_function(
                World::Utility,
                "function() { return document.fonts.ready.then(() => undefined); }",
                &[],
                timeout,
                "waiting for fonts",
            )
            .await;
        let full_page = if o.full_page {
            let v = self
                .call_function(
                    World::Utility,
                    FULL_PAGE_SIZE,
                    &[],
                    timeout,
                    "measuring the page",
                )
                .await?;
            if v.is_null() {
                return Err(Error::Evaluation("document has no body".into()));
            }
            Some(Size {
                width: num(&v, "width"),
                height: num(&v, "height"),
            })
        } else {
            None
        };
        let target = plan_page(o, viewport, full_page)?;
        let transparent = o.omit_background && o.format != ImageFormat::Jpeg;
        if transparent {
            self.send(
                "Emulation.setDefaultBackgroundColorOverride",
                json!({"color": {"r": 0, "g": 0, "b": 0, "a": 0}}),
            )
            .await?;
        }
        let shot = self.capture_target(o, quality, target).await;
        if transparent {
            let _ = self
                .send("Emulation.setDefaultBackgroundColorOverride", json!({}))
                .await;
        }
        shot
    }

    async fn capture_target(
        &self,
        o: &ScreenshotOptions,
        quality: Option<u8>,
        target: crate::domain::screenshot::Target,
    ) -> Result<Vec<u8>> {
        let metrics = self.send("Page.getLayoutMetrics", json!({})).await?;
        let vv = &metrics["visualViewport"];
        let vv = VisualViewport {
            page_x: num(vv, "pageX"),
            page_y: num(vv, "pageY"),
            scale: num(vv, "scale"),
        };
        let css_ratio = self
            .inner
            .options
            .device_scale_factor
            .filter(|d| *d > 0.0)
            .or_else(|| {
                let w = num(&metrics["contentSize"], "width");
                let css = num(&metrics["cssContentSize"], "width");
                (css > 0.0).then(|| w / css)
            })
            .filter(|r| *r > 0.0)
            .unwrap_or(1.0);
        let c = capture(target, vv, o.scale, css_ratio);
        let format = match o.format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Webp => "webp",
        };
        let mut params = json!({
            "format": format,
            "clip": {"x": c.clip.x, "y": c.clip.y, "width": c.clip.width, "height": c.clip.height, "scale": c.scale},
            "captureBeyondViewport": c.capture_beyond_viewport,
        });
        if let Some(q) = quality {
            params["quality"] = json!(q);
        }
        if o.format == ImageFormat::Png {
            params["optimizeForSpeed"] = json!(true);
        }
        let r = self.send("Page.captureScreenshot", params).await?;
        let data = r.get("data").and_then(Value::as_str).unwrap_or_default();
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| Error::Serde(format!("screenshot data: {e}")))
    }
}

fn num(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or_default()
}

/// A pending `page.screenshot()`.
#[must_use = "does nothing until awaited"]
pub struct Screenshot<'a> {
    page: &'a Page,
    options: ScreenshotOptions,
    path: Option<PathBuf>,
    timeout: Option<Duration>,
}

impl<'a> Screenshot<'a> {
    pub fn full_page(mut self, yes: bool) -> Self {
        self.options.full_page = yes;
        self
    }
    pub fn clip(mut self, clip: Rect) -> Self {
        self.options.clip = Some(clip);
        self
    }
    pub fn omit_background(mut self, yes: bool) -> Self {
        self.options.omit_background = yes;
        self
    }
    pub fn format(mut self, format: ImageFormat) -> Self {
        self.options.format = format;
        self
    }
    pub fn quality(mut self, quality: u8) -> Self {
        self.options.quality = Some(quality);
        self
    }
    pub fn scale(mut self, scale: ScreenshotScale) -> Self {
        self.options.scale = scale;
        self
    }
    pub fn caret(mut self, caret: Caret) -> Self {
        self.options.caret = caret;
        self
    }
    pub fn animations(mut self, animations: Animations) -> Self {
        self.options.animations = animations;
        self
    }
    /// Stylesheet applied while capturing.
    pub fn style(mut self, css: &str) -> Self {
        self.options.style = Some(css.to_owned());
        self
    }
    /// Also write the image to this file.
    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
    /// All options at once.
    pub fn options(mut self, options: ScreenshotOptions) -> Self {
        self.options = options;
        self
    }
}

impl<'a> IntoFuture for Screenshot<'a> {
    type Output = Result<Vec<u8>>;
    type IntoFuture = BoxFuture<'a, Result<Vec<u8>>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let timeout = self.timeout.unwrap_or_else(|| self.page.default_timeout());
            let bytes = self.page.run_screenshot(&self.options, timeout).await?;
            if let Some(path) = &self.path {
                self.page.inner.shared.services.fs.write(path, &bytes)?;
            }
            Ok(bytes)
        }
        .boxed()
    }
}
