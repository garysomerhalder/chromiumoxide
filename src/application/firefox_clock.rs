//! `context.clock` / `page.clock` on Firefox: Playwright's fake timers (its
//! vendored clock script, as on Chromium), as a preload script for new
//! documents and evaluated in every open frame.

use std::sync::atomic::Ordering;

use serde_json::json;

use crate::application::clock::CLOCK;
use crate::application::firefox::{BrowserContext, Page};
use crate::domain::clock::{ClockTime, Ticks, parse_ticks, parse_time};
use crate::domain::error::{Error, Result};

/// Controls time in a Firefox context (Playwright's `Clock`). Cheap to clone.
#[derive(Clone, Debug)]
pub struct Clock {
    context: BrowserContext,
}

impl BrowserContext {
    /// `context.clock`.
    pub fn clock(&self) -> Clock {
        Clock {
            context: self.clone(),
        }
    }
}

impl Page {
    /// `page.clock`: the clock of the page's context.
    pub fn clock(&self) -> Clock {
        Clock {
            context: self
                .context()
                .expect("a page belongs to a context while it is open"),
        }
    }
}

impl Clock {
    fn now(&self) -> f64 {
        self.context.inner.services.clock.wall_ms()
    }

    async fn evaluate_in_frames(&self, script: &str) -> Result<()> {
        for page in self.context.live_pages() {
            for frame in page.frames().await.unwrap_or_default() {
                let r = page
                    .evaluate_value_in(json!({"context": frame.context}), script, None)
                    .await;
                match r {
                    Ok(_) | Err(Error::TargetClosed(_)) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(())
    }

    async fn install_if_needed(&self) -> Result<()> {
        if self
            .context
            .inner
            .clock_installed
            .swap(true, Ordering::SeqCst)
        {
            return Ok(());
        }
        let script = format!(
            "(() => {{\n  const module = {{}};\n  {CLOCK}\n  if (!globalThis.__pwClock)\n    globalThis.__pwClock = (module.exports.inject())(globalThis, \"firefox\");\n}})()"
        );
        self.context.add_init_script(&script).await?;
        self.evaluate_in_frames(&script).await
    }

    /// Records the call for new documents, then runs it in every frame.
    async fn call(&self, method: &str, arg: Option<f64>) -> Result<()> {
        self.install_if_needed().await?;
        let now = self.now();
        let log_arg = arg.map(|a| format!(", {a}")).unwrap_or_default();
        self.context
            .add_init_script(&format!(
                "globalThis.__pwClock.controller.log('{method}', {now}{log_arg})"
            ))
            .await?;
        let arg = arg.map(|a| a.to_string()).unwrap_or_default();
        self.evaluate_in_frames(&format!(
            "(async () => {{ await globalThis.__pwClock.controller.{method}({arg}); }})()"
        ))
        .await
    }

    /// `clock.install({ time })`: fake timers from `time` (default now).
    pub async fn install(&self, time: Option<ClockTime>) -> Result<()> {
        let t = match time {
            Some(t) => parse_time(&t)?,
            None => self.now(),
        };
        self.call("install", Some(t)).await
    }

    /// `clock.fastForward(ticks)`: jumps ahead, firing due timers at most once.
    pub async fn fast_forward(&self, ticks: impl Into<Ticks>) -> Result<()> {
        let ms = parse_ticks(&ticks.into())?;
        self.call("fastForward", Some(ms)).await
    }

    /// `clock.pauseAt(time)`: runs timers up to `time`, then pauses.
    pub async fn pause_at(&self, time: impl Into<ClockTime>) -> Result<()> {
        let t = parse_time(&time.into())?;
        self.call("pauseAt", Some(t)).await
    }

    /// `clock.resume()`.
    pub async fn resume(&self) -> Result<()> {
        self.call("resume", None).await
    }

    /// `clock.runFor(ticks)`: advances time, firing every due timer.
    pub async fn run_for(&self, ticks: impl Into<Ticks>) -> Result<()> {
        let ms = parse_ticks(&ticks.into())?;
        self.call("runFor", Some(ms)).await
    }

    /// `clock.setFixedTime(time)`: `Date.now()` always returns `time`.
    pub async fn set_fixed_time(&self, time: impl Into<ClockTime>) -> Result<()> {
        let t = parse_time(&time.into())?;
        self.call("setFixedTime", Some(t)).await
    }

    /// `clock.setSystemTime(time)`: changes the time without firing timers.
    pub async fn set_system_time(&self, time: impl Into<ClockTime>) -> Result<()> {
        let t = parse_time(&time.into())?;
        self.call("setSystemTime", Some(t)).await
    }
}
