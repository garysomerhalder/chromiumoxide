//! `expect(page | locator).toHaveScreenshot(path)`: compares a stable
//! screenshot with a saved PNG, following Playwright's `SnapshotHelper` and
//! `page._expectScreenshot`.

use std::future::IntoFuture;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;

use crate::application::expect::{LocatorAssertions, PageAssertions};
use crate::application::locator::Locator;
use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::expect::DEFAULT_EXPECT_TIMEOUT_MS;
use crate::domain::image::{CompareOptions, compare_images, decode_png, encode_png, validate};
use crate::domain::screenshot::{Animations, Caret, Rect, ScreenshotOptions, ScreenshotScale};

/// Playwright's `updateSnapshots` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpdateSnapshots {
    /// Write snapshots that do not exist yet (the assertion still fails).
    #[default]
    Missing,
    /// Never write snapshots.
    None,
    /// Rewrite every snapshot.
    All,
    /// Rewrite snapshots that do not match.
    Changed,
}

/// A pending `toHaveScreenshot`; chain options, then `.await` it.
#[must_use = "assertions do nothing until awaited"]
pub struct ScreenshotAssertion<'a> {
    page: &'a Page,
    locator: Option<Locator>,
    is_not: bool,
    path: PathBuf,
    options: ScreenshotOptions,
    mask: Vec<Locator>,
    mask_color: Option<String>,
    compare: CompareOptions,
    update: UpdateSnapshots,
    output_dir: Option<PathBuf>,
    timeout: Option<Duration>,
}

impl<'a> PageAssertions<'a> {
    /// `expect(page).toHaveScreenshot(path)`.
    pub fn to_have_screenshot(&self, path: impl AsRef<Path>) -> ScreenshotAssertion<'a> {
        ScreenshotAssertion::new(self.page(), None, self.is_negated(), path.as_ref())
    }
}

impl<'a> LocatorAssertions<'a> {
    /// `expect(locator).toHaveScreenshot(path)`.
    pub fn to_have_screenshot(&self, path: impl AsRef<Path>) -> ScreenshotAssertion<'a> {
        let locator = self.locator();
        ScreenshotAssertion::new(
            &locator.page,
            Some(locator.clone()),
            self.is_negated(),
            path.as_ref(),
        )
    }
}

impl<'a> ScreenshotAssertion<'a> {
    fn new(page: &'a Page, locator: Option<Locator>, is_not: bool, path: &Path) -> Self {
        Self {
            page,
            locator,
            is_not,
            path: path.to_owned(),
            // Playwright's toHaveScreenshot defaults.
            options: ScreenshotOptions {
                animations: Animations::Disabled,
                caret: Caret::Hide,
                scale: ScreenshotScale::Css,
                ..Default::default()
            },
            mask: Vec::new(),
            mask_color: None,
            compare: CompareOptions::default(),
            update: UpdateSnapshots::default(),
            output_dir: None,
            timeout: None,
        }
    }

    pub fn max_diff_pixels(mut self, pixels: u64) -> Self {
        self.compare.max_diff_pixels = Some(pixels);
        self
    }
    pub fn max_diff_pixel_ratio(mut self, ratio: f64) -> Self {
        self.compare.max_diff_pixel_ratio = Some(ratio);
        self
    }
    /// Per-pixel colour tolerance, 0..=1 (default 0.2).
    pub fn threshold(mut self, threshold: f64) -> Self {
        self.compare.threshold = Some(threshold);
        self
    }
    pub fn mask(mut self, locators: &[Locator]) -> Self {
        self.mask.extend(locators.iter().cloned());
        self
    }
    pub fn mask_color(mut self, color: &str) -> Self {
        self.mask_color = Some(color.to_owned());
        self
    }
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
    pub fn animations(mut self, animations: Animations) -> Self {
        self.options.animations = animations;
        self
    }
    pub fn caret(mut self, caret: Caret) -> Self {
        self.options.caret = caret;
        self
    }
    pub fn scale(mut self, scale: ScreenshotScale) -> Self {
        self.options.scale = scale;
        self
    }
    /// Stylesheet applied while capturing (Playwright's `stylePath` content).
    pub fn style(mut self, css: &str) -> Self {
        self.options.style = Some(css.to_owned());
        self
    }
    /// When to write the snapshot file (default: only when missing).
    pub fn update(mut self, update: UpdateSnapshots) -> Self {
        self.update = update;
        self
    }
    /// Where to write `-actual`, `-expected`, `-previous` and `-diff` images
    /// when the assertion fails (not written without it).
    pub fn output_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.output_dir = Some(dir.into());
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

/// What `_expectScreenshot` returns.
#[derive(Default)]
struct Outcome {
    actual: Option<Vec<u8>>,
    previous: Option<Vec<u8>>,
    diff: Option<Vec<u8>>,
    error: Option<String>,
}

impl ScreenshotAssertion<'_> {
    fn fs(&self) -> &dyn crate::ports::FileSystem {
        &*self.page.inner.shared.services.fs
    }

    async fn shoot(&self, timeout: Duration) -> Result<Vec<u8>> {
        let shot = match &self.locator {
            Some(l) => l.screenshot_builder(),
            None => self.page.screenshot(),
        };
        let mut shot = shot
            .options(self.options.clone())
            .mask(&self.mask)
            .timeout(timeout);
        if let Some(c) = &self.mask_color {
            shot = shot.mask_color(c);
        }
        shot.await
    }

    /// Whether two screenshots count as "equal" for the loop: matching, or
    /// different under `.not`. Records the latest mismatch.
    fn equal(
        &self,
        actual: &[u8],
        expected: Option<&[u8]>,
        previous: Option<&[u8]>,
        last: &mut Option<Outcome>,
    ) -> Result<bool> {
        let Some(expected) = expected else {
            return Ok(false);
        };
        let mismatch = compare_images(&decode_png(actual)?, &decode_png(expected)?, &self.compare);
        if mismatch.is_some() == self.is_not {
            return Ok(true);
        }
        if let Some(m) = mismatch {
            *last = Some(Outcome {
                actual: Some(actual.to_vec()),
                previous: previous.map(<[u8]>::to_vec),
                diff: Some(encode_png(&m.diff)),
                error: Some(m.message),
            });
        }
        Ok(false)
    }

    /// Playwright's `page._expectScreenshot`.
    async fn expect_screenshot(
        &self,
        expected: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<Outcome> {
        let clock = self.page.inner.shared.services.clock.clone();
        let deadline = clock.now_ms() + timeout.as_millis() as u64;
        let mut intervals = [0u64, 100, 250, 500].into_iter();
        let mut actual: Option<Vec<u8>> = None;
        let mut last: Option<Outcome> = None;
        let mut first = true;
        loop {
            if self.page.is_closed() {
                return Err(Error::TargetClosed("The page has closed".into()));
            }
            let wait = intervals.next().unwrap_or(1000);
            let now = clock.now_ms();
            if now >= deadline || (wait > 0 && now + wait >= deadline) {
                let error = match last.take() {
                    Some(o) if o.previous.is_some() => Outcome {
                        error: Some("Failed to take two consecutive stable screenshots.".into()),
                        ..o
                    },
                    Some(o) => Outcome {
                        error: Some(format!("Timeout {}ms exceeded.", timeout.as_millis())),
                        ..o
                    },
                    None => Outcome {
                        error: Some(format!("Timeout {}ms exceeded.", timeout.as_millis())),
                        ..Default::default()
                    },
                };
                return Ok(error);
            }
            if wait > 0 {
                clock.sleep(Duration::from_millis(wait)).await;
            }
            let previous = actual.take();
            let remaining = Duration::from_millis(deadline.saturating_sub(clock.now_ms()).max(1));
            let shot = match self.shoot(remaining).await {
                Ok(s) => s,
                Err(e @ (Error::TargetClosed(_) | Error::InvalidArgument(_))) => return Err(e),
                Err(_) => {
                    actual = previous;
                    continue;
                }
            };
            let expectation = match expected {
                Some(e) if first => Some(e),
                _ => previous.as_deref(),
            };
            let done = self.equal(&shot, expectation, previous.as_deref(), &mut last)?;
            actual = Some(shot);
            if done {
                break;
            }
            first = false;
        }
        let actual = actual.expect("set before breaking");
        let Some(expected) = expected else {
            return Ok(Outcome {
                actual: Some(actual),
                ..Default::default()
            });
        };
        if first || self.equal(&actual, Some(expected), None, &mut last)? {
            return Ok(Outcome::default());
        }
        Ok(last.unwrap_or_else(|| Outcome {
            error: Some("Screenshot comparison failed".into()),
            ..Default::default()
        }))
    }

    async fn header(&self) -> String {
        match &self.locator {
            Some(l) => {
                let name = l.to_playwright_string().await.unwrap_or_default();
                format!("expect(locator).toHaveScreenshot(expected) failed\n\nLocator: {name}\n")
            }
            None => "expect(page).toHaveScreenshot(expected) failed\n\n".to_owned(),
        }
    }

    fn output_path(&self, suffix: &str) -> Option<PathBuf> {
        let dir = self.output_dir.as_ref()?;
        let stem = self.path.file_stem()?.to_string_lossy();
        Some(dir.join(format!("{stem}{suffix}.png")))
    }

    fn write_output(&self, suffix: &str, data: Option<&[u8]>) -> Result<()> {
        if let (Some(path), Some(data)) = (self.output_path(suffix), data) {
            self.fs().write(&path, data)?;
        }
        Ok(())
    }

    /// Playwright's `handleDifferent`.
    async fn different(&self, o: Outcome, expected: Option<&[u8]>, extra: &str) -> Error {
        let error = o.error.unwrap_or_default();
        let indented: String = error
            .lines()
            .map(|l| {
                if l.is_empty() {
                    String::new()
                } else {
                    format!("  {l}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let message = format!(
            "{}{extra}{indented}\n\n  Snapshot: {}",
            self.header().await,
            self.path.display()
        );
        let written = self
            .write_output("-expected", expected)
            .and(self.write_output("-previous", o.previous.as_deref()))
            .and(self.write_output("-actual", o.actual.as_deref()))
            .and(self.write_output("-diff", o.diff.as_deref()));
        match written {
            Ok(()) => Error::Assertion(message),
            Err(e) => e,
        }
    }

    async fn run(self) -> Result<()> {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !name.ends_with(".png") {
            if name.ends_with(".webp") {
                return Err(Error::InvalidArgument(
                    "webp snapshots are not supported yet; use a '.png' name".into(),
                ));
            }
            return Err(Error::InvalidArgument(format!(
                "Screenshot name \"{name}\" must have a '.png' or '.webp' extension"
            )));
        }
        validate(&self.compare)?;
        let timeout = self
            .timeout
            .unwrap_or(Duration::from_millis(DEFAULT_EXPECT_TIMEOUT_MS));
        let shown = self.path.display().to_string();
        let existing = self.fs().read(&self.path).ok();
        if self.is_not {
            let Some(expected) = existing else {
                return Err(Error::Assertion(format!(
                    "A snapshot doesn't exist at {shown}, matchers using \".not\" won't write them automatically."
                )));
            };
            let o = self.expect_screenshot(Some(&expected), timeout).await?;
            if o.error.is_none() {
                return Ok(());
            }
            return Err(Error::Assertion(
                "Screenshot comparison failed:\n\n  Expected result should be different from the actual one.".into(),
            ));
        }
        let Some(expected) = existing else {
            if self.update == UpdateSnapshots::None {
                return Err(Error::Assertion(format!(
                    "A snapshot doesn't exist at {shown}."
                )));
            }
            let o = self.expect_screenshot(None, timeout).await?;
            if o.error.is_some() {
                return Err(self.different(o, None, "").await);
            }
            let actual = o.actual.unwrap_or_default();
            self.fs().write(&self.path, &actual)?;
            self.write_output("-actual", Some(&actual))?;
            if matches!(self.update, UpdateSnapshots::All | UpdateSnapshots::Changed) {
                return Ok(());
            }
            return Err(Error::Assertion(format!(
                "A snapshot doesn't exist at {shown}, writing actual."
            )));
        };
        let compare_to = (self.update != UpdateSnapshots::All).then_some(expected.as_slice());
        let o = self.expect_screenshot(compare_to, timeout).await?;
        if o.error.is_none() {
            if let Some(actual) = o.actual.as_ref().filter(|a| **a != expected) {
                self.fs().write(&self.path, actual)?;
            }
            return Ok(());
        }
        if matches!(self.update, UpdateSnapshots::All | UpdateSnapshots::Changed) {
            if let Some(actual) = &o.actual {
                self.fs().write(&self.path, actual)?;
                return Ok(());
            }
            return Err(self
                .different(o, compare_to, "  Failed to re-generate expected.\n")
                .await);
        }
        Err(self.different(o, compare_to, "").await)
    }
}

impl<'a> IntoFuture for ScreenshotAssertion<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        self.run().boxed()
    }
}
