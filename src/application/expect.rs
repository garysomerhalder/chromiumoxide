//! Web-first assertions (Playwright's `expect`): they retry until the page
//! matches or the timeout (5 s by default) elapses, using Playwright's
//! injected `expect()` for the checks.

use std::future::IntoFuture;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::{Value, json};

use crate::application::locator::Locator;
use crate::application::network::Response;
use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::expect::{
    DEFAULT_EXPECT_TIMEOUT_MS, Failure, Subject, TextOptions, display_expected, expected_text,
    poll_interval_ms, quote,
};
use crate::domain::selectors::TextMatch;

/// Something `expect()` can make assertions about.
pub trait Expect {
    type Assertions<'a>
    where
        Self: 'a;
    fn assertions(&self) -> Self::Assertions<'_>;
}

/// `expect(locator)`, `expect(page)` or `expect(response)`.
pub fn expect<T: Expect + ?Sized>(target: &T) -> T::Assertions<'_> {
    target.assertions()
}

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

impl Expect for Response {
    type Assertions<'a> = ResponseAssertions<'a>;
    fn assertions(&self) -> ResponseAssertions<'_> {
        ResponseAssertions {
            response: self,
            is_not: false,
        }
    }
}

enum Check {
    /// Through the injected `expect()` with these options.
    Injected { options: Value, array: bool },
    /// `toHaveJSProperty`: compared in the page's main world.
    JsProperty { name: String, value: Value },
}

/// How "Received:" is printed.
#[derive(Clone, Copy)]
enum Show {
    Quoted,
    Plain,
    Json,
}

/// A pending assertion: `.await` it; `.timeout()` changes the 5 s default.
#[must_use = "assertions do nothing until awaited"]
pub struct Assertion<'a> {
    locator: Locator,
    subject: Subject,
    check: Check,
    matcher: String,
    expected: String,
    show: Show,
    is_not: bool,
    timeout: Option<Duration>,
    _page: std::marker::PhantomData<&'a ()>,
}

impl<'a> Assertion<'a> {
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
  const r = await i.expect(elements[0], options, elements);
  return { missing: false, matches: r.matches, received: r.received ? r.received.value : undefined };
}";

impl Assertion<'_> {
    async fn once(&self, timeout: Duration) -> Result<Outcome> {
        let is_not = self.is_not;
        match &self.check {
            Check::Injected { options, array } => {
                let missing = |expression: &str| -> Outcome {
                    let expected_len = options
                        .get("expectedText")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    let (matches, received, not_found) = match expression {
                        "to.have.count" => (
                            options.get("expectedNumber").and_then(Value::as_u64) == Some(0),
                            Some(json!(0)),
                            false,
                        ),
                        e if e.ends_with(".array") => (expected_len == 0, Some(json!([])), false),
                        "to.be.hidden" if !is_not => (true, None, false),
                        "to.be.visible" if is_not => (false, None, false),
                        "to.be.detached" if !is_not => (true, None, false),
                        "to.be.attached" if is_not => (false, None, false),
                        "to.be.in.viewport" if is_not => (false, None, false),
                        _ => (is_not, None, true),
                    };
                    Outcome {
                        matches,
                        received,
                        not_found,
                    }
                };
                let expression = options
                    .get("expression")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let Some((frame, selector, _)) = self.locator.scope(timeout).await? else {
                    return Ok(missing(&expression));
                };
                let r = self
                    .locator
                    .page()
                    .injected_call(
                        frame.as_deref(),
                        RUN,
                        &[json!(selector), json!(array), options.clone()],
                        true,
                        timeout,
                    )
                    .await?;
                if r.get("missing").and_then(Value::as_bool) == Some(true) {
                    return Ok(missing(&expression));
                }
                Ok(Outcome {
                    matches: r.get("matches").and_then(Value::as_bool) == Some(true),
                    received: r.get("received").cloned(),
                    not_found: false,
                })
            }
            Check::JsProperty { name, value } => {
                if self.locator.count().await? == 0 {
                    return Ok(Outcome {
                        matches: is_not,
                        received: None,
                        not_found: true,
                    });
                }
                let got: Value = self
                    .locator
                    .evaluate_with("(e, name) => e[name]", name)
                    .await?;
                Ok(Outcome {
                    matches: &got == value,
                    received: Some(got),
                    not_found: false,
                })
            }
        }
    }

    async fn run(self) -> Result<()> {
        let page = self.locator.page().clone();
        let clock = page.inner.shared.services.clock.clone();
        let timeout = self
            .timeout
            .unwrap_or(Duration::from_millis(DEFAULT_EXPECT_TIMEOUT_MS));
        let start = clock.now_ms();
        let deadline = start + timeout.as_millis() as u64;
        let mut attempt = 0usize;
        let mut last: Option<Outcome> = None;
        loop {
            let remaining = Duration::from_millis(deadline.saturating_sub(clock.now_ms()).max(1));
            match self.once(remaining).await {
                Ok(o) => {
                    if o.matches != self.is_not {
                        return Ok(());
                    }
                    last = Some(o);
                }
                Err(e) if crate::application::js::is_context_lost(&e) => {}
                Err(Error::Timeout { .. }) => {}
                Err(e) => return Err(e),
            }
            let wait = poll_interval_ms(attempt);
            attempt += 1;
            if clock.now_ms() + wait >= deadline {
                break;
            }
            clock.sleep(Duration::from_millis(wait)).await;
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

/// Assertions about a locator.
pub struct LocatorAssertions<'a> {
    locator: &'a Locator,
    is_not: bool,
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

impl<'a> LocatorAssertions<'a> {
    pub(crate) fn locator(&self) -> &'a Locator {
        self.locator
    }
    pub(crate) fn is_negated(&self) -> bool {
        self.is_not
    }

    /// The negated assertion (`expect(locator).not`).
    #[allow(clippy::should_implement_trait)] // mirrors Playwright's `.not`
    pub fn not(mut self) -> Self {
        self.is_not = !self.is_not;
        self
    }

    fn make(
        &self,
        options: Value,
        array: bool,
        matcher: &str,
        expected: String,
        show: Show,
    ) -> Assertion<'a> {
        let mut options = options;
        options["isNot"] = json!(self.is_not);
        Assertion {
            locator: self.locator.clone(),
            subject: Subject::Locator(format!("locator('{}')", self.locator.selector())),
            check: Check::Injected { options, array },
            matcher: matcher.to_owned(),
            expected,
            show,
            is_not: self.is_not,
            timeout: None,
            _page: std::marker::PhantomData,
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
        if let Check::Injected { options, .. } = &mut a.check {
            options["expectedValue"] = json!({"checked": true});
        }
        a
    }
    pub fn to_be_unchecked(&self) -> Assertion<'a> {
        let mut a = self.state("to.be.checked", "toBeChecked()", "unchecked");
        if let Check::Injected { options, .. } = &mut a.check {
            options["expectedValue"] = json!({"checked": false});
        }
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
    pub fn to_be_in_viewport(&self) -> Assertion<'a> {
        self.state("to.be.in.viewport", "toBeInViewport()", "in viewport")
    }
    /// `toBeInViewport({ratio})`.
    pub fn to_be_in_viewport_ratio(&self, ratio: f64) -> Assertion<'a> {
        let mut a = self.to_be_in_viewport();
        if let Check::Injected { options, .. } = &mut a.check {
            options["expectedNumber"] = json!(ratio);
        }
        a
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
    /// `toHaveAttribute(name)`: the attribute is present.
    pub fn to_have_attribute_present(&self, name: &str) -> Assertion<'a> {
        let expected = if self.is_not {
            "not have attribute"
        } else {
            "have attribute"
        };
        self.make(
            json!({"expression": "to.have.attribute", "expressionArg": name}),
            false,
            "toHaveAttribute(expected)",
            expected.to_owned(),
            Show::Plain,
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
    /// `toHaveValues([...])` for `<select multiple>`.
    pub fn to_have_values<I, T>(&self, values: I) -> Assertion<'a>
    where
        I: IntoIterator<Item = T>,
        T: Into<TextMatch>,
    {
        let (values, display) = texts(values, TextOptions::EXACT);
        self.make(
            json!({"expression": "to.have.values", "expectedText": values}),
            false,
            "toHaveValues(expected)",
            display,
            Show::Json,
        )
    }
    pub fn to_have_accessible_name(&self, name: impl Into<TextMatch>) -> Assertion<'a> {
        self.text(
            "to.have.accessible.name",
            "toHaveAccessibleName(expected)",
            name.into(),
            TextOptions::HAVE,
            None,
        )
    }
    pub fn to_have_accessible_description(&self, text: impl Into<TextMatch>) -> Assertion<'a> {
        self.text(
            "to.have.accessible.description",
            "toHaveAccessibleDescription(expected)",
            text.into(),
            TextOptions::HAVE,
            None,
        )
    }
    pub fn to_have_accessible_error_message(&self, text: impl Into<TextMatch>) -> Assertion<'a> {
        self.text(
            "to.have.accessible.error.message",
            "toHaveAccessibleErrorMessage(expected)",
            text.into(),
            TextOptions::HAVE,
            None,
        )
    }
    /// `toHaveJSProperty(name, value)`, compared as JSON.
    pub fn to_have_js_property<V: Serialize>(&self, name: &str, value: V) -> Assertion<'a> {
        let value = serde_json::to_value(value).unwrap_or(Value::Null);
        Assertion {
            locator: self.locator.clone(),
            subject: Subject::Locator(format!("locator('{}')", self.locator.selector())),
            expected: value.to_string(),
            check: Check::JsProperty {
                name: name.to_owned(),
                value,
            },
            matcher: "toHaveJSProperty(expected)".into(),
            show: Show::Json,
            is_not: self.is_not,
            timeout: None,
            _page: std::marker::PhantomData,
        }
    }
}

/// Assertions about a page.
pub struct PageAssertions<'a> {
    page: &'a Page,
    is_not: bool,
}

impl<'a> PageAssertions<'a> {
    pub(crate) fn page(&self) -> &'a Page {
        self.page
    }
    pub(crate) fn is_negated(&self) -> bool {
        self.is_not
    }

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
            check: Check::Injected {
                options: json!({"expression": expression, "isNot": self.is_not,
                                "expectedText": [expected_text(&expected, o)]}),
                array: false,
            },
            matcher: matcher.to_owned(),
            expected: display_expected(&expected),
            show: Show::Quoted,
            is_not: self.is_not,
            timeout: None,
            _page: std::marker::PhantomData,
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

/// Assertions about a response.
pub struct ResponseAssertions<'a> {
    response: &'a Response,
    is_not: bool,
}

impl<'a> ResponseAssertions<'a> {
    #[allow(clippy::should_implement_trait)] // mirrors Playwright's `.not`
    pub fn not(mut self) -> Self {
        self.is_not = !self.is_not;
        self
    }

    /// `toBeOK()`: status 200..=299.
    pub fn to_be_ok(&self) -> BoxFuture<'a, Result<()>> {
        let response = self.response;
        let is_not = self.is_not;
        async move {
            if response.ok() != is_not {
                return Ok(());
            }
            let not = if is_not { "not." } else { "" };
            Err(Error::Assertion(format!(
                "expect(response).{not}toBeOK() failed\n\nExpected: {}\nReceived: {} {}\nURL: {}",
                if is_not { "not OK" } else { "OK" },
                response.status,
                response.status_text,
                response.url
            )))
        }
        .boxed()
    }
}
