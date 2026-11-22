//! `expect(page | locator).toMatchAriaSnapshot(template)`: Playwright's
//! template matching through the injected script, with its retries, update
//! modes and failure messages.

use std::future::IntoFuture;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::application::expect::{LocatorAssertions, PageAssertions};
use crate::application::locator::Locator;
use crate::application::snapshot::UpdateSnapshots;
use crate::domain::aria_template::{parse_aria_template, unshift};
use crate::domain::error::{Error, Result};
use crate::domain::expect::{
    DEFAULT_EXPECT_TIMEOUT_MS, line_diff, poll_interval_ms, quote_keep_lines,
};

const MATCH: &str = "async (i, selector, template, frameVisible) => {
  const e = i.querySelector(i.parseSelector(selector), document, true);
  if (!e) return { missing: true };
  const r = await i.expect(e, { expression: 'to.match.aria', expectedValue: template, isNot: false }, [e], frameVisible);
  const v = r.received && r.received.value;
  return { missing: false, matches: !!r.matches, raw: v ? v.raw : '', regex: v ? v.regex : '' };
}";

/// Where the expected template comes from.
enum Source {
    Text(String),
    File(PathBuf),
}

/// A pending `toMatchAriaSnapshot`; `.await` it.
#[must_use = "assertions do nothing until awaited"]
pub struct AriaAssertion<'a> {
    locator: Locator,
    is_page: bool,
    is_not: bool,
    source: Source,
    update: UpdateSnapshots,
    timeout: Option<Duration>,
    _page: std::marker::PhantomData<&'a ()>,
}

impl<'a> PageAssertions<'a> {
    /// `expect(page).toMatchAriaSnapshot(template)`: the page's body.
    pub fn to_match_aria_snapshot(&self, template: &str) -> AriaAssertion<'a> {
        AriaAssertion::new(
            self.page().locator("body"),
            true,
            self.is_negated(),
            Source::Text(template.to_owned()),
        )
    }
    /// `expect(page).toMatchAriaSnapshot({ name })` with a template file.
    pub fn to_match_aria_snapshot_file(&self, path: impl AsRef<Path>) -> AriaAssertion<'a> {
        AriaAssertion::new(
            self.page().locator("body"),
            true,
            self.is_negated(),
            Source::File(path.as_ref().to_owned()),
        )
    }
}

impl<'a> LocatorAssertions<'a> {
    /// `expect(locator).toMatchAriaSnapshot(template)`.
    pub fn to_match_aria_snapshot(&self, template: &str) -> AriaAssertion<'a> {
        AriaAssertion::new(
            self.locator().clone(),
            false,
            self.is_negated(),
            Source::Text(template.to_owned()),
        )
    }
    /// `expect(locator).toMatchAriaSnapshot({ name })` with a template file.
    pub fn to_match_aria_snapshot_file(&self, path: impl AsRef<Path>) -> AriaAssertion<'a> {
        AriaAssertion::new(
            self.locator().clone(),
            false,
            self.is_negated(),
            Source::File(path.as_ref().to_owned()),
        )
    }
}

struct Outcome {
    missing: bool,
    matches: bool,
    raw: String,
    regex: String,
}

impl<'a> AriaAssertion<'a> {
    fn new(locator: Locator, is_page: bool, is_not: bool, source: Source) -> Self {
        Self {
            locator,
            is_page,
            is_not,
            source,
            update: UpdateSnapshots::default(),
            timeout: None,
            _page: std::marker::PhantomData,
        }
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// When to write a template file (default: only when missing).
    pub fn update(mut self, update: UpdateSnapshots) -> Self {
        self.update = update;
        self
    }

    async fn once(&self, template: &Value, timeout: Duration) -> Result<Outcome> {
        let Some((frame, selector, _, frame_visible)) = self.locator.scope_visible(timeout).await?
        else {
            return Ok(Outcome {
                missing: true,
                matches: false,
                raw: String::new(),
                regex: String::new(),
            });
        };
        let r = self
            .locator
            .page
            .injected_call(
                frame.as_deref(),
                MATCH,
                &[json!(selector), template.clone(), json!(frame_visible)],
                true,
                timeout,
            )
            .await?;
        let text = |k: &str| {
            r.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Ok(Outcome {
            missing: r.get("missing").and_then(Value::as_bool) == Some(true),
            matches: r.get("matches").and_then(Value::as_bool) == Some(true),
            raw: text("raw"),
            regex: text("regex"),
        })
    }

    async fn header(&self) -> (String, Option<String>) {
        let not = if self.is_not { "not." } else { "" };
        let who = if self.is_page { "page" } else { "locator" };
        let head = format!("expect({who}).{not}toMatchAriaSnapshot(expected) failed\n\n");
        let locator = if self.is_page {
            None
        } else {
            Some(
                self.locator
                    .to_playwright_string()
                    .await
                    .unwrap_or_default(),
            )
        };
        (head, locator)
    }

    async fn run(self) -> Result<()> {
        let timeout = self
            .timeout
            .unwrap_or(Duration::from_millis(DEFAULT_EXPECT_TIMEOUT_MS));
        let fs = self.locator.page.inner.shared.services.fs.clone();
        let expected = match &self.source {
            Source::Text(t) => t.clone(),
            Source::File(p) => fs
                .read(p)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default(),
        };
        let missing_baseline = self.update == UpdateSnapshots::Missing && expected.is_empty();
        if missing_baseline && self.is_not {
            return Err(Error::Assertion(
                "Matchers using \".not\" can't generate new baselines".into(),
            ));
        }
        let generate = !self.is_not && (self.update == UpdateSnapshots::All || missing_baseline);
        let expected = unshift(&expected);
        let clock = self.locator.page.inner.shared.services.clock.clone();
        let start = clock.now_ms();
        let template = if generate {
            json!({"kind": "role", "role": "none", "name": "Generating new baseline"})
        } else {
            match parse_aria_template(&expected) {
                Ok(t) => t,
                Err(e) => {
                    let (head, locator) = self.header().await;
                    let not = if self.is_not { "not " } else { "" };
                    let locator = locator
                        .map(|l| format!("Locator: {l}\n"))
                        .unwrap_or_default();
                    return Err(Error::Assertion(format!(
                        "{head}{locator}Expected: {not}{}\nError: {}",
                        quote_keep_lines(&expected),
                        e.trim_end()
                    )));
                }
            }
        };
        let mut attempt = 0;
        let outcome = loop {
            let remaining = timeout.saturating_sub(Duration::from_millis(clock.now_ms() - start));
            let o = self
                .once(&template, remaining.max(Duration::from_millis(1)))
                .await?;
            let passed = !o.missing && o.matches != self.is_not;
            if passed || generate {
                break o;
            }
            let wait = poll_interval_ms(attempt);
            attempt += 1;
            if clock.now_ms() + wait >= start + timeout.as_millis() as u64 {
                break o;
            }
            clock.sleep(Duration::from_millis(wait)).await;
        };
        let pass = !outcome.missing && outcome.matches;
        if !self.is_not
            && !outcome.missing
            && (generate || (self.update == UpdateSnapshots::Changed && !pass))
        {
            return match &self.source {
                Source::File(path) => {
                    fs.write(path, outcome.regex.as_bytes())?;
                    if missing_baseline {
                        Err(Error::Assertion(format!(
                            "A snapshot doesn't exist at {}, writing actual.",
                            path.display()
                        )))
                    } else {
                        Ok(())
                    }
                }
                Source::Text(_) if missing_baseline => Err(Error::Assertion(format!(
                    "A snapshot is not provided, generating new baseline.\n\n{}",
                    outcome.regex
                ))),
                Source::Text(_) => Ok(()),
            };
        }
        if !outcome.missing && pass != self.is_not {
            return Ok(());
        }
        let (head, locator) = self.header().await;
        let ms = timeout.as_millis();
        if outcome.missing {
            let locator = locator
                .map(|l| format!("Locator: {l}\n"))
                .unwrap_or_default();
            return Err(Error::Assertion(format!(
                "{head}{locator}Expected: {}\nTimeout: {ms}ms\nError: element(s) not found",
                quote_keep_lines(&expected)
            )));
        }
        if self.is_not {
            let locator = locator
                .map(|l| format!("Locator:  {l}\n"))
                .unwrap_or_default();
            return Err(Error::Assertion(format!(
                "{head}{locator}Expected: not {}\nReceived: {}\nTimeout:  {ms}ms",
                quote_keep_lines(&expected),
                quote_keep_lines(&outcome.raw)
            )));
        }
        let locator = locator
            .map(|l| format!("Locator: {l}\n"))
            .unwrap_or_default();
        Err(Error::Assertion(format!(
            "{head}{locator}Timeout: {ms}ms\n{}",
            line_diff(&expected, &outcome.raw)
        )))
    }
}

impl<'a> IntoFuture for AriaAssertion<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        self.run().boxed()
    }
}
