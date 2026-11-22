//! `page.coverage`: JavaScript and CSS coverage (Playwright's `CRCoverage`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use futures::channel::oneshot;
use serde_json::{Value, json};

use crate::application::page::Page;
use crate::domain::coverage::{CountedRange, Range, disjoint_ranges};
use crate::domain::error::{Error, Result};

/// Options of `startJSCoverage` / `startCSSCoverage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverageOptions {
    /// Forget what was collected when the page navigates (default true).
    pub reset_on_navigation: bool,
    /// Also report scripts without a URL (JS only; default false).
    pub report_anonymous_scripts: bool,
}

impl Default for CoverageOptions {
    fn default() -> Self {
        Self {
            reset_on_navigation: true,
            report_anonymous_scripts: false,
        }
    }
}

/// One script of `stopJSCoverage()`.
#[derive(Debug, Clone, PartialEq)]
pub struct JsCoverageEntry {
    pub url: String,
    pub script_id: String,
    pub source: Option<String>,
    /// V8's per-function block coverage (`functions` of the profile).
    pub functions: Value,
}

/// One stylesheet of `stopCSSCoverage()`.
#[derive(Debug, Clone, PartialEq)]
pub struct CssCoverageEntry {
    pub url: String,
    pub text: Option<String>,
    /// The used parts of `text`.
    pub ranges: Vec<Range>,
}

#[derive(Default)]
struct Collected {
    /// Script / stylesheet id → URL.
    urls: HashMap<String, String>,
    sources: HashMap<String, String>,
    ids: HashSet<String>,
}

struct Running {
    stop: oneshot::Sender<()>,
    data: Arc<Mutex<Collected>>,
    options: CoverageOptions,
}

#[derive(Default)]
pub(crate) struct CoverageState {
    js: Option<Running>,
    css: Option<Running>,
}

/// `page.coverage`.
#[derive(Clone, Debug)]
pub struct Coverage {
    page: Page,
}

impl Page {
    /// `page.coverage`.
    pub fn coverage(&self) -> Coverage {
        Coverage { page: self.clone() }
    }
}

impl Coverage {
    /// Collects script ids and sources (or stylesheet texts) until stopped.
    fn collect(&self, css: bool, options: CoverageOptions) -> Running {
        let (stop, mut stop_rx) = oneshot::channel();
        let data = Arc::new(Mutex::new(Collected::default()));
        let page = self.page.clone();
        let mut events = page.inner.shared.cdp.subscribe();
        let collected = data.clone();
        tokio::spawn(async move {
            loop {
                let e = tokio::select! {
                    _ = &mut stop_rx => break,
                    e = events.next() => match e { Some(e) => e, None => break },
                };
                if e.session_id.as_deref() != Some(page.inner.session_id.as_str()) {
                    continue;
                }
                let p = &e.params;
                let text = |v: &Value, k: &str| {
                    v.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                match e.method.as_str() {
                    "Runtime.executionContextsCleared" if options.reset_on_navigation => {
                        *collected.lock().unwrap() = Collected::default();
                    }
                    "Debugger.paused" if !css => {
                        let _ = page.send("Debugger.resume", json!({})).await;
                    }
                    "Debugger.scriptParsed" if !css => {
                        let id = text(p, "scriptId");
                        let url = text(p, "url");
                        collected.lock().unwrap().ids.insert(id.clone());
                        if url.is_empty() && !options.report_anonymous_scripts {
                            continue;
                        }
                        let (page, collected) = (page.clone(), collected.clone());
                        tokio::spawn(async move {
                            if let Ok(r) = page
                                .send("Debugger.getScriptSource", json!({"scriptId": id}))
                                .await
                            {
                                let source = r
                                    .get("scriptSource")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned();
                                collected.lock().unwrap().sources.insert(id, source);
                            }
                        });
                    }
                    "CSS.styleSheetAdded" if css => {
                        let header = &p["header"];
                        let url = text(header, "sourceURL");
                        if url.is_empty() {
                            continue;
                        }
                        let id = text(header, "styleSheetId");
                        let (page, collected) = (page.clone(), collected.clone());
                        tokio::spawn(async move {
                            if let Ok(r) = page
                                .send("CSS.getStyleSheetText", json!({"styleSheetId": id}))
                                .await
                            {
                                let source = r
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned();
                                let mut c = collected.lock().unwrap();
                                c.urls.insert(id.clone(), url);
                                c.sources.insert(id, source);
                            }
                        });
                    }
                    _ => {}
                }
            }
        });
        Running {
            stop,
            data,
            options,
        }
    }

    /// `coverage.startJSCoverage(options)`.
    pub async fn start_js_coverage(&self, options: CoverageOptions) -> Result<()> {
        {
            let st = self.page.inner.coverage.lock().unwrap();
            if st.js.is_some() {
                return Err(Error::InvalidArgument(
                    "JSCoverage is already enabled".into(),
                ));
            }
        }
        let running = self.collect(false, options);
        self.page.inner.coverage.lock().unwrap().js = Some(running);
        let p = &self.page;
        let (a, b, c, d) = futures::join!(
            p.send("Profiler.enable", json!({})),
            p.send(
                "Profiler.startPreciseCoverage",
                json!({"callCount": true, "detailed": true})
            ),
            p.send("Debugger.enable", json!({})),
            p.send("Debugger.setSkipAllPauses", json!({"skip": true})),
        );
        a?;
        b?;
        c?;
        d?;
        Ok(())
    }

    /// `coverage.stopJSCoverage()`: the scripts with their function ranges.
    pub async fn stop_js_coverage(&self) -> Result<Vec<JsCoverageEntry>> {
        let Some(running) = self.page.inner.coverage.lock().unwrap().js.take() else {
            return Ok(Vec::new());
        };
        let p = &self.page;
        let (profile, _, _, _) = futures::join!(
            p.send("Profiler.takePreciseCoverage", json!({})),
            p.send("Profiler.stopPreciseCoverage", json!({})),
            p.send("Profiler.disable", json!({})),
            p.send("Debugger.disable", json!({})),
        );
        let _ = running.stop.send(());
        let profile = profile?;
        let data = running.data.lock().unwrap();
        let mut entries = Vec::new();
        for entry in profile["result"].as_array().into_iter().flatten() {
            let id = entry
                .get("scriptId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
            if !data.ids.contains(id)
                || (url.is_empty() && !running.options.report_anonymous_scripts)
            {
                continue;
            }
            entries.push(JsCoverageEntry {
                url: url.to_owned(),
                script_id: id.to_owned(),
                source: data.sources.get(id).cloned(),
                functions: entry.get("functions").cloned().unwrap_or(json!([])),
            });
        }
        Ok(entries)
    }

    /// `coverage.startCSSCoverage(options)`.
    pub async fn start_css_coverage(&self, options: CoverageOptions) -> Result<()> {
        {
            let st = self.page.inner.coverage.lock().unwrap();
            if st.css.is_some() {
                return Err(Error::InvalidArgument(
                    "CSSCoverage is already enabled".into(),
                ));
            }
        }
        let running = self.collect(true, options);
        self.page.inner.coverage.lock().unwrap().css = Some(running);
        let p = &self.page;
        let (a, b, c) = futures::join!(
            p.send("DOM.enable", json!({})),
            p.send("CSS.enable", json!({})),
            p.send("CSS.startRuleUsageTracking", json!({})),
        );
        a?;
        b?;
        c?;
        Ok(())
    }

    /// `coverage.stopCSSCoverage()`: the stylesheets with their used ranges.
    pub async fn stop_css_coverage(&self) -> Result<Vec<CssCoverageEntry>> {
        let Some(running) = self.page.inner.coverage.lock().unwrap().css.take() else {
            return Ok(Vec::new());
        };
        let usage = self.page.send("CSS.stopRuleUsageTracking", json!({})).await;
        let _ = futures::join!(
            self.page.send("CSS.disable", json!({})),
            self.page.send("DOM.disable", json!({})),
        );
        let _ = running.stop.send(());
        let usage = usage?;
        let mut by_sheet: HashMap<String, Vec<CountedRange>> = HashMap::new();
        for u in usage["ruleUsage"].as_array().into_iter().flatten() {
            let id = u
                .get("styleSheetId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let num = |k: &str| u.get(k).and_then(Value::as_f64).unwrap_or_default() as u64;
            by_sheet
                .entry(id.to_owned())
                .or_default()
                .push(CountedRange {
                    start: num("startOffset"),
                    end: num("endOffset"),
                    count: u64::from(u.get("used").and_then(Value::as_bool) == Some(true)),
                });
        }
        let data = running.data.lock().unwrap();
        Ok(data
            .urls
            .iter()
            .map(|(id, url)| CssCoverageEntry {
                url: url.clone(),
                text: data.sources.get(id).cloned(),
                ranges: disjoint_ranges(by_sheet.get(id).map_or(&[][..], Vec::as_slice)),
            })
            .collect())
    }
}
