//! Web-first assertions on the WebKit engine: `expect(&locator)` and
//! `expect(&page)` with Playwright's injected `expect()` checks, retried
//! until they match or the timeout (5 s by default) elapses. As
//! `application::firefox_expect`, on the WebKit page's utility world.

use std::future::IntoFuture;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::application::expect::Expect;
use crate::application::webkit::Page;
use crate::application::webkit_locator::Locator;
use crate::domain::error::{Error, Result};
use crate::domain::expect::{
    DEFAULT_EXPECT_TIMEOUT_MS, Failure, Subject, TextOptions, display_expected, expected_text,
    poll_interval_ms, quote,
};
use crate::domain::selectors::TextMatch;

impl Expect for Locator {
    type Assertions<'a> = LocatorAssertions<'a>;
    fn assertions(&self) -> LocatorAssertions<'_> {
        LocatorAssertions {
            locator: self,
            is_not: false,
        }
    }
}

impl Expect for Page {
    type Assertions<'a> = PageAssertions<'a>;
    fn assertions(&self) -> PageAssertions<'_> {
        PageAssertions {
            page: self,
            is_not: false,
        }
    }
}

/// How "Received:" is printed.
#[derive(Clone, Copy)]
enum Show {
    Quoted,
    Plain,
    Json,
}

/// A pending WebKit assertion: `.await` it; `.timeout()` changes the 5 s
/// default.
#[must_use = "assertions do nothing until awaited"]
pub struct Assertion<'a> {
    locator: Locator,
    subject: Subject,
    options: Value,
    array: bool,
    matcher: String,
    expected: String,
    show: Show,
    is_not: bool,
    timeout: Option<Duration>,
    _lifetime: std::marker::PhantomData<&'a ()>,
}

impl Assertion<'_> {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

struct Outcome {
    matches: bool,
    received: Option<Value>,
    not_found: bool,
}

const RUN: &str = "async (i, selector, isArray, options) => {
  const parsed = i.parseSelector(selector);
  let elements;
  if (isArray) elements = i.querySelectorAll(parsed, document);
  else { const e = i.querySelector(parsed, document, true); elements = e ? [e] : []; }
  if (!elements.length) return { missing: true };
  const r = await i.expect(elements[0], options, elements, true);
  return { missing: false, matches: r.matches, received: r.received ? r.received.value : undefined };
}";

impl Assertion<'_> {
    /// Playwright's result for a selector matching nothing.
    fn missing(&self) -> Outcome {
        let expression = self.options["expression"].as_str().unwrap_or_default();
        let expected_len = self.options["expectedText"].as_array().map_or(0, Vec::len);
        let is_not = self.is_not;
        let (matches, received, not_found) = match expression {
            "to.have.count" => (
                self.options["expectedNumber"].as_u64() == Some(0),
                Some(json!(0)),
                false,
            ),
            e if e.ends_with(".array") => (expected_len == 0, Some(json!([])), false),
            "to.be.hidden" if !is_not => (true, None, false),
            "to.be.visible" if is_not => (false, None, false),
            "to.be.detached" if !is_not => (true, None, false),
            "to.be.attached" if is_not => (false, None, false),
            _ => (is_not, None, true),
        };
        Outcome {
            matches,
            received,
            not_found,
        }
    }

    async fn once(&self) -> Result<Outcome> {
        let r = self
            .locator
            .call(
                RUN,
                &[
                    json!(self.locator.selector()),
                    json!(self.array),
                    self.options.clone(),
                ],
            )
            .await?;
        if r.get("missing").and_then(Value::as_bool) == Some(true) {
            return Ok(self.missing());
        }
        Ok(Outcome {
            matches: r.get("matches").and_then(Value::as_bool) == Some(true),
            received: r.get("received").cloned(),
            not_found: false,
        })
    }

    async fn run(self) -> Result<()> {
        let page = self.locator.page().clone();
        let timeout = self
            .timeout
            .unwrap_or(Duration::from_millis(DEFAULT_EXPECT_TIMEOUT_MS));
        let deadline = page.now() + timeout;
        let mut attempt = 0usize;
        let mut last: Option<Outcome> = None;
        loop {
            match self.once().await {
                Ok(o) => {
                    if o.matches != self.is_not {
                        return Ok(());
                    }
                    last = Some(o);
                }
                // The realm went away (a navigation): try again.
                Err(Error::Protocol { .. }) => {}
                Err(e) => return Err(e),
            }
            let wait = Duration::from_millis(poll_interval_ms(attempt));
            attempt += 1;
            if page.now() + wait >= deadline {
                break;
            }
            page.sleep(wait).await;
        }
        let (received, not_found) = match last {
            Some(o) => (o.received, o.not_found),
            None => (None, true),
        };
        let received = received.map(|r| match (self.show, &r) {
            (Show::Quoted, Value::String(s)) => quote(s),
            (Show::Plain, Value::String(s)) => s.clone(),
            _ => r.to_string(),
        });
        Err(Error::Assertion(
            Failure {
                subject: self.subject,
                matcher: self.matcher,
                is_not: self.is_not,
                expected: self.expected,
                received,
                timeout_ms: timeout.as_millis() as u64,
                not_found,
            }
            .message(),
        ))
    }
}

impl<'a> IntoFuture for Assertion<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        self.run().boxed()
    }
}

fn texts<I, T>(items: I, o: TextOptions) -> (Vec<Value>, String)
where
    I: IntoIterator<Item = T>,
    T: Into<TextMatch>,
{
    let items: Vec<TextMatch> = items.into_iter().map(Into::into).collect();
    let display = format!(
        "[{}]",
        items
            .iter()
            .map(display_expected)
            .collect::<Vec<_>>()
            .join(", ")
    );
    (items.iter().map(|m| expected_text(m, o)).collect(), display)
}

/// Assertions about a WebKit locator.
pub struct LocatorAssertions<'a> {
    locator: &'a Locator,
    is_not: bool,
}

impl<'a> LocatorAssertions<'a> {
    /// The negated assertion (`expect(locator).not`).
    #[allow(clippy::should_implement_trait)] // mirrors Playwright's `.not`
    pub fn not(mut self) -> Self {
        self.is_not = !self.is_not;
        self
    }

    fn make(
        &self,
        mut options: Value,
        array: bool,
        matcher: &str,
        expected: String,
        show: Show,
    ) -> Assertion<'a> {
        options["isNot"] = json!(self.is_not);
        Assertion {
            locator: self.locator.clone(),
            subject: Subject::Locator(format!("locator('{}')", self.locator.selector())),
            options,
            array,
            matcher: matcher.to_owned(),
            expected,
            show,
            is_not: self.is_not,
            timeout: None,
            _lifetime: std::marker::PhantomData,
        }
    }

    fn state(&self, expression: &str, matcher: &str, word: &str) -> Assertion<'a> {
        let expected = if self.is_not {
            format!("not {word}")
        } else {
            word.to_owned()
        };
        self.make(
            json!({"expression": expression}),
            false,
            matcher,
            expected,
            Show::Plain,
        )
    }

    fn text(
        &self,
        expression: &str,
        matcher: &str,
        expected: TextMatch,
        o: TextOptions,
        arg: Option<&str>,
    ) -> Assertion<'a> {
        let mut options =
            json!({"expression": expression, "expectedText": [expected_text(&expected, o)]});
        if let Some(a) = arg {
            options["expressionArg"] = json!(a);
        }
        self.make(
            options,
            false,
            matcher,
            display_expected(&expected),
            Show::Quoted,
        )
    }

    pub fn to_be_attached(&self) -> Assertion<'a> {
        self.state("to.be.attached", "toBeAttached()", "attached")
    }
    pub fn to_be_detached(&self) -> Assertion<'a> {
        self.state("to.be.detached", "toBeAttached()", "detached")
    }
    pub fn to_be_checked(&self) -> Assertion<'a> {
        let mut a = self.state("to.be.checked", "toBeChecked()", "checked");
        a.options["expectedValue"] = json!({"checked": true});
        a
    }
    pub fn to_be_unchecked(&self) -> Assertion<'a> {
        let mut a = self.state("to.be.checked", "toBeChecked()", "unchecked");
        a.options["expectedValue"] = json!({"checked": false});
        a
    }
    pub fn to_be_disabled(&self) -> Assertion<'a> {
        self.state("to.be.disabled", "toBeDisabled()", "disabled")
    }
    pub fn to_be_editable(&self) -> Assertion<'a> {
        self.state("to.be.editable", "toBeEditable()", "editable")
    }
    pub fn to_be_empty(&self) -> Assertion<'a> {
        self.state("to.be.empty", "toBeEmpty()", "empty")
    }
    pub fn to_be_enabled(&self) -> Assertion<'a> {
        self.state("to.be.enabled", "toBeEnabled()", "enabled")
    }
    pub fn to_be_focused(&self) -> Assertion<'a> {
        self.state("to.be.focused", "toBeFocused()", "focused")
    }
    pub fn to_be_hidden(&self) -> Assertion<'a> {
        self.state("to.be.hidden", "toBeHidden()", "hidden")
    }
    pub fn to_be_visible(&self) -> Assertion<'a> {
        self.state("to.be.visible", "toBeVisible()", "visible")
    }
    pub fn to_have_text(&self, expected: impl Into<TextMatch>) -> Assertion<'a> {
        self.text(
            "to.have.text",
            "toHaveText(expected)",
            expected.into(),
            TextOptions::HAVE,
            None,
        )
    }
    pub fn to_contain_text(&self, expected: impl Into<TextMatch>) -> Assertion<'a> {
        self.text(
            "to.have.text",
            "toContainText(expected)",
            expected.into(),
            TextOptions::CONTAIN,
            None,
        )
    }
    /// `toHaveText([...])`: one entry per matching element.
    pub fn to_have_texts<I, T>(&self, expected: I) -> Assertion<'a>
    where
        I: IntoIterator<Item = T>,
        T: Into<TextMatch>,
    {
        let (values, display) = texts(expected, TextOptions::HAVE);
        self.make(
            json!({"expression": "to.have.text.array", "expectedText": values}),
            true,
            "toHaveText(expected)",
            display,
            Show::Json,
        )
    }
    /// `toContainText([...])`.
    pub fn to_contain_texts<I, T>(&self, expected: I) -> Assertion<'a>
    where
        I: IntoIterator<Item = T>,
        T: Into<TextMatch>,
    {
        let (values, display) = texts(expected, TextOptions::CONTAIN);
        self.make(
            json!({"expression": "to.contain.text.array", "expectedText": values}),
            true,
            "toContainText(expected)",
            display,
            Show::Json,
        )
    }
    pub fn to_have_attribute(&self, name: &str, value: impl Into<TextMatch>) -> Assertion<'a> {
        self.text(
            "to.have.attribute.value",
            "toHaveAttribute(expected)",
            value.into(),
            TextOptions::EXACT,
            Some(name),
        )
    }
    pub fn to_have_class(&self, expected: impl Into<TextMatch>) -> Assertion<'a> {
        self.text(
            "to.have.class",
            "toHaveClass(expected)",
            expected.into(),
            TextOptions::EXACT,
            None,
        )
    }
    pub fn to_contain_class(&self, expected: &str) -> Assertion<'a> {
        self.text(
            "to.contain.class",
            "toContainClass(expected)",
            TextMatch::Text(expected.to_owned()),
            TextOptions::EXACT,
            None,
        )
    }
    pub fn to_have_count(&self, count: usize) -> Assertion<'a> {
        self.make(
            json!({"expression": "to.have.count", "expectedNumber": count}),
            true,
            "toHaveCount(expected)",
            count.to_string(),
            Show::Json,
        )
    }
    pub fn to_have_css(&self, name: &str, value: impl Into<TextMatch>) -> Assertion<'a> {
        self.text(
            "to.have.css",
            "toHaveCSS(expected)",
            value.into(),
            TextOptions::EXACT,
            Some(name),
        )
    }
    pub fn to_have_id(&self, id: impl Into<TextMatch>) -> Assertion<'a> {
        self.text(
            "to.have.id",
            "toHaveId(expected)",
            id.into(),
            TextOptions::EXACT,
            None,
        )
    }
    pub fn to_have_role(&self, role: &str) -> Assertion<'a> {
        self.text(
            "to.have.role",
            "toHaveRole(expected)",
            TextMatch::Text(role.to_owned()),
            TextOptions::EXACT,
            None,
        )
    }
    pub fn to_have_value(&self, value: impl Into<TextMatch>) -> Assertion<'a> {
        self.text(
            "to.have.value",
            "toHaveValue(expected)",
            value.into(),
            TextOptions::EXACT,
            None,
        )
    }
}

/// Assertions about a WebKit page.
pub struct PageAssertions<'a> {
    page: &'a Page,
    is_not: bool,
}

impl<'a> PageAssertions<'a> {
    #[allow(clippy::should_implement_trait)] // mirrors Playwright's `.not`
    pub fn not(mut self) -> Self {
        self.is_not = !self.is_not;
        self
    }

    fn page_text(
        &self,
        expression: &str,
        matcher: &str,
        expected: TextMatch,
        o: TextOptions,
    ) -> Assertion<'a> {
        Assertion {
            locator: self.page.locator(":root"),
            subject: Subject::Page,
            options: json!({"expression": expression, "isNot": self.is_not,
                            "expectedText": [expected_text(&expected, o)]}),
            array: false,
            matcher: matcher.to_owned(),
            expected: display_expected(&expected),
            show: Show::Quoted,
            is_not: self.is_not,
            timeout: None,
            _lifetime: std::marker::PhantomData,
        }
    }

    pub fn to_have_title(&self, title: impl Into<TextMatch>) -> Assertion<'a> {
        self.page_text(
            "to.have.title",
            "toHaveTitle(expected)",
            title.into(),
            TextOptions::HAVE,
        )
    }
    pub fn to_have_url(&self, url: impl Into<TextMatch>) -> Assertion<'a> {
        self.page_text(
            "to.have.url",
            "toHaveURL(expected)",
            url.into(),
            TextOptions::EXACT,
        )
    }
}
