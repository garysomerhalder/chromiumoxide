//! Screenshot geometry, as Playwright computes it (`screenshotter.ts`, `crPage.ts`).

/// A rectangle in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// A size in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// Image format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
    Webp,
}

/// `scale` option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScreenshotScale {
    Css,
    #[default]
    Device,
}

/// `caret` option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Caret {
    #[default]
    Hide,
    Initial,
}

/// `animations` option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Animations {
    #[default]
    Allow,
    Disabled,
}

/// Playwright's `page.screenshot(options)`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScreenshotOptions {
    pub format: ImageFormat,
    pub quality: Option<u8>,
    pub full_page: bool,
    pub clip: Option<Rect>,
    pub omit_background: bool,
    pub scale: ScreenshotScale,
    pub caret: Caret,
    pub animations: Animations,
    /// Extra stylesheet applied while capturing.
    pub style: Option<String>,
}

/// The visual viewport part of `Page.getLayoutMetrics`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualViewport {
    pub page_x: f64,
    pub page_y: f64,
    pub scale: f64,
}

/// Where to capture: the result of planning, before reading layout metrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// A rectangle of the viewport (viewport screenshots, with or without clip).
    Viewport(Rect),
    /// A rectangle of the document (full page, element).
    Document { rect: Rect, fits_viewport: bool },
}

/// What `Page.captureScreenshot` receives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Capture {
    pub clip: Rect,
    pub scale: f64,
    pub capture_beyond_viewport: bool,
}

/// Validation error message (Playwright's wording).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ScreenshotError(pub String);

/// Validates options; returns the quality to send (jpeg 80 / webp 100 defaults).
pub fn validate(o: &ScreenshotOptions) -> Result<Option<u8>, ScreenshotError> {
    let fail = |m: String| Err(ScreenshotError(m));
    if let Some(q) = o.quality {
        if o.format == ImageFormat::Png {
            return fail("options.quality is unsupported for the png screenshots".into());
        }
        if q > 100 {
            return fail(format!(
                "Expected options.quality to be between 0 and 100 (inclusive), got {q}"
            ));
        }
    }
    if let Some(clip) = o.clip {
        if clip.width <= 0.0 {
            return fail("Expected options.clip.width to be greater than 0.".into());
        }
        if clip.height <= 0.0 {
            return fail("Expected options.clip.height to be greater than 0.".into());
        }
    }
    Ok(match o.format {
        ImageFormat::Png => None,
        ImageFormat::Jpeg => Some(o.quality.unwrap_or(80)),
        ImageFormat::Webp => Some(o.quality.unwrap_or(100)),
    })
}

/// Playwright's `trimClipToSize`.
pub fn trim_clip_to_size(clip: Rect, size: Size) -> Result<Rect, ScreenshotError> {
    let x1 = clip.x.min(size.width).max(0.0);
    let y1 = clip.y.min(size.height).max(0.0);
    let x2 = (clip.x + clip.width).min(size.width).max(0.0);
    let y2 = (clip.y + clip.height).min(size.height).max(0.0);
    let r = Rect::new(x1, y1, x2 - x1, y2 - y1);
    if r.width > 0.0 && r.height > 0.0 {
        Ok(r)
    } else {
        Err(ScreenshotError(
            "Clipped area is either empty or outside the resulting image".into(),
        ))
    }
}

/// Decides the capture target for a page screenshot. `full_page` is the
/// document size and is only needed when `o.full_page` is set.
pub fn plan_page(
    o: &ScreenshotOptions,
    viewport: Size,
    full_page: Option<Size>,
) -> Result<Target, ScreenshotError> {
    if o.full_page {
        let size = full_page.ok_or_else(|| ScreenshotError("full page size is unknown".into()))?;
        let fits_viewport = size.width <= viewport.width && size.height <= viewport.height;
        let mut rect = Rect::new(0.0, 0.0, size.width, size.height);
        if let Some(clip) = o.clip {
            rect = trim_clip_to_size(clip, size)?;
        }
        return Ok(Target::Document {
            rect,
            fits_viewport,
        });
    }
    let rect = match o.clip {
        Some(clip) => trim_clip_to_size(clip, viewport)?,
        None => Rect::new(0.0, 0.0, viewport.width, viewport.height),
    };
    Ok(Target::Viewport(rect))
}

/// Turns a target into the capture parameters using the live layout metrics.
/// `css_ratio` is the device scale factor used for `scale: css`.
pub fn capture(
    target: Target,
    vv: VisualViewport,
    scale: ScreenshotScale,
    css_ratio: f64,
) -> Capture {
    let (clip, mut factor, capture_beyond_viewport) = match target {
        Target::Viewport(r) => (
            Rect::new(
                vv.page_x + r.x,
                vv.page_y + r.y,
                (r.width / vv.scale + 1e-3).floor(),
                (r.height / vv.scale + 1e-3).floor(),
            ),
            vv.scale,
            false,
        ),
        Target::Document {
            rect,
            fits_viewport,
        } => (rect, 1.0, !fits_viewport),
    };
    if scale == ScreenshotScale::Css {
        factor /= if css_ratio > 0.0 { css_ratio } else { 1.0 };
    }
    Capture {
        clip,
        scale: factor,
        capture_beyond_viewport,
    }
}

/// Playwright's `enclosingIntRect`.
pub fn enclosing_int_rect(r: Rect) -> Rect {
    let x = (r.x + 1e-3).floor();
    let y = (r.y + 1e-3).floor();
    let x2 = (r.x + r.width - 1e-3).ceil();
    let y2 = (r.y + r.height - 1e-3).ceil();
    Rect::new(x, y, x2 - x, y2 - y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VV: VisualViewport = VisualViewport {
        page_x: 0.0,
        page_y: 0.0,
        scale: 0.5102040767669678,
    };
    const VIEW: Size = Size {
        width: 500.0,
        height: 400.0,
    };

    fn opts() -> ScreenshotOptions {
        ScreenshotOptions::default()
    }

    #[test]
    fn viewport_screenshot_matches_recorded_playwright_traffic() {
        let t = plan_page(&opts(), VIEW, None).unwrap();
        assert_eq!(t, Target::Viewport(Rect::new(0.0, 0.0, 500.0, 400.0)));
        let c = capture(t, VV, ScreenshotScale::Device, 2.0);
        assert_eq!(c.clip, Rect::new(0.0, 0.0, 980.0, 784.0));
        assert_eq!(c.scale, 0.5102040767669678);
        assert!(!c.capture_beyond_viewport);
    }

    #[test]
    fn clip_screenshot_matches_recorded_playwright_traffic() {
        let o = ScreenshotOptions {
            clip: Some(Rect::new(10.0, 10.0, 100.0, 50.0)),
            ..opts()
        };
        let c = capture(
            plan_page(&o, VIEW, None).unwrap(),
            VV,
            ScreenshotScale::Device,
            2.0,
        );
        assert_eq!(c.clip, Rect::new(10.0, 10.0, 196.0, 98.0));
        assert_eq!(c.scale, 0.5102040767669678);
        assert!(!c.capture_beyond_viewport);
    }

    #[test]
    fn full_page_matches_recorded_playwright_traffic() {
        let o = ScreenshotOptions {
            full_page: true,
            ..opts()
        };
        let t = plan_page(
            &o,
            VIEW,
            Some(Size {
                width: 980.0,
                height: 2000.0,
            }),
        )
        .unwrap();
        assert_eq!(
            t,
            Target::Document {
                rect: Rect::new(0.0, 0.0, 980.0, 2000.0),
                fits_viewport: false
            }
        );
        let c = capture(t, VV, ScreenshotScale::Device, 2.0);
        assert_eq!(c.clip, Rect::new(0.0, 0.0, 980.0, 2000.0));
        assert_eq!(c.scale, 1.0);
        assert!(c.capture_beyond_viewport);
    }

    #[test]
    fn full_page_that_fits_does_not_capture_beyond_viewport() {
        let o = ScreenshotOptions {
            full_page: true,
            ..opts()
        };
        let t = plan_page(
            &o,
            VIEW,
            Some(Size {
                width: 500.0,
                height: 300.0,
            }),
        )
        .unwrap();
        assert!(!capture(t, VV, ScreenshotScale::Device, 1.0).capture_beyond_viewport);
    }

    #[test]
    fn full_page_with_clip_is_trimmed_to_the_document() {
        let o = ScreenshotOptions {
            full_page: true,
            clip: Some(Rect::new(900.0, 1900.0, 500.0, 500.0)),
            ..opts()
        };
        let t = plan_page(
            &o,
            VIEW,
            Some(Size {
                width: 980.0,
                height: 2000.0,
            }),
        )
        .unwrap();
        assert_eq!(
            t,
            Target::Document {
                rect: Rect::new(900.0, 1900.0, 80.0, 100.0),
                fits_viewport: false
            }
        );
    }

    #[test]
    fn css_scale_divides_by_device_scale_factor() {
        let vv = VisualViewport {
            page_x: 0.0,
            page_y: 0.0,
            scale: 1.0,
        };
        let o = ScreenshotOptions {
            full_page: true,
            ..opts()
        };
        let t = plan_page(
            &o,
            VIEW,
            Some(Size {
                width: 500.0,
                height: 300.0,
            }),
        )
        .unwrap();
        assert_eq!(capture(t, vv, ScreenshotScale::Css, 3.0).scale, 1.0 / 3.0);
        let t = plan_page(&opts(), VIEW, None).unwrap();
        assert_eq!(capture(t, vv, ScreenshotScale::Css, 2.0).scale, 0.5);
    }

    #[test]
    fn scrolled_visual_viewport_offsets_the_clip() {
        let vv = VisualViewport {
            page_x: 5.0,
            page_y: 300.0,
            scale: 1.0,
        };
        let c = capture(
            plan_page(&opts(), VIEW, None).unwrap(),
            vv,
            ScreenshotScale::Device,
            1.0,
        );
        assert_eq!(c.clip, Rect::new(5.0, 300.0, 500.0, 400.0));
    }

    /// Chrome on Android lays a page without `<meta name=viewport>` out 980
    /// CSS px wide and zooms out to fit the screen (scale < 1). Then
    /// `innerWidth` / `innerHeight` already are the visible area; dividing
    /// by the scale (right when zoomed in) made the clip 2.7x too large and
    /// Chrome tiled the page 3x3. Numbers from a real emulator (Chrome 145,
    /// 1080x1944 screen, DPR 3).
    #[test]
    fn zoomed_out_mobile_page_captures_the_visible_area() {
        let vv = VisualViewport {
            page_x: 0.0,
            page_y: 0.0,
            scale: 0.36734694242477417,
        };
        let inner = Size {
            width: 980.0,
            height: 1764.0,
        };
        let c = capture(
            plan_page(&opts(), inner, None).unwrap(),
            vv,
            ScreenshotScale::Device,
            3.0,
        );
        assert_eq!(c.clip, Rect::new(0.0, 0.0, 980.0, 1764.0));
        assert_eq!(c.scale, 0.36734694242477417);
    }

    #[test]
    fn trimming_rejects_empty_areas() {
        let err = trim_clip_to_size(Rect::new(600.0, 0.0, 10.0, 10.0), VIEW).unwrap_err();
        assert_eq!(
            err.0,
            "Clipped area is either empty or outside the resulting image"
        );
        assert_eq!(
            trim_clip_to_size(Rect::new(-10.0, -10.0, 50.0, 50.0), VIEW).unwrap(),
            Rect::new(0.0, 0.0, 40.0, 40.0)
        );
    }

    #[test]
    fn validation_messages_and_quality_defaults() {
        assert_eq!(validate(&opts()).unwrap(), None);
        let jpeg = ScreenshotOptions {
            format: ImageFormat::Jpeg,
            ..opts()
        };
        assert_eq!(validate(&jpeg).unwrap(), Some(80));
        let webp = ScreenshotOptions {
            format: ImageFormat::Webp,
            ..opts()
        };
        assert_eq!(validate(&webp).unwrap(), Some(100));
        let bad = ScreenshotOptions {
            quality: Some(50),
            ..opts()
        };
        assert_eq!(
            validate(&bad).unwrap_err().0,
            "options.quality is unsupported for the png screenshots"
        );
        let bad = ScreenshotOptions {
            format: ImageFormat::Jpeg,
            quality: Some(101),
            ..opts()
        };
        assert_eq!(
            validate(&bad).unwrap_err().0,
            "Expected options.quality to be between 0 and 100 (inclusive), got 101"
        );
        let bad = ScreenshotOptions {
            clip: Some(Rect::new(0.0, 0.0, 0.0, 5.0)),
            ..opts()
        };
        assert_eq!(
            validate(&bad).unwrap_err().0,
            "Expected options.clip.width to be greater than 0."
        );
        let bad = ScreenshotOptions {
            clip: Some(Rect::new(0.0, 0.0, 5.0, -1.0)),
            ..opts()
        };
        assert_eq!(
            validate(&bad).unwrap_err().0,
            "Expected options.clip.height to be greater than 0."
        );
    }

    #[test]
    fn enclosing_int_rect_like_playwright() {
        assert_eq!(
            enclosing_int_rect(Rect::new(1.2, 2.9995, 10.5, 3.0)),
            Rect::new(1.0, 3.0, 11.0, 3.0)
        );
    }
}
