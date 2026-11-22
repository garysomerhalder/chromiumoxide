# chromiumoxide

A Chrome DevTools Protocol client for Rust with a Playwright-like API.

This is a fork of [mattsse/chromiumoxide](https://github.com/mattsse/chromiumoxide),
rewritten for 0.10 as domain-driven, hexagonal code (see
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)). It targets parity with
Playwright for Chromium; progress is tracked in [`docs/PARITY.md`](docs/PARITY.md).
Screenshots are tested to be pixel-identical to Playwright 1.63's.

## Usage

```rust
use chromiumoxide::{Chromium, ContextOptions, LaunchOptions, Viewport, WaitUntil};

#[tokio::main]
async fn main() -> chromiumoxide::Result<()> {
    // Finds Chromium like Playwright does (PLAYWRIGHT_BROWSERS_PATH, ~/.cache/ms-playwright, ...)
    let browser = Chromium::launch(LaunchOptions::default()).await?;
    let context = browser
        .new_context(ContextOptions {
            viewport: Some(Viewport::new(390, 844)),
            device_scale_factor: Some(3.0),
            is_mobile: true,
            ..Default::default()
        })
        .await?;
    let page = context.new_page().await?;
    page.goto("https://example.com").wait_until(WaitUntil::NetworkIdle).await?;
    let title: String = page.evaluate("() => document.title").await?;
    page.screenshot().full_page(true).path("example.png").await?;
    println!("{title}");
    browser.close().await
}
```

Device presets come from Playwright: `chromiumoxide::device("iPhone 15").unwrap().context_options()`.

### Finding the browser

In order: `LaunchOptions::executable_path`, `$MOCKUP_CHROMIUM`, `$CHROME`,
`$PLAYWRIGHT_BROWSERS_PATH`, `/opt/pw-browsers`, `~/.cache/ms-playwright`,
`~/Library/Caches/ms-playwright` (headless shell before full Chromium, newest
first), Chrome for Testing caches (`chromiumoxide_fetcher`, puppeteer), then a
system Chrome. Install a browser with `npx playwright install chromium-headless-shell`.

Each launch gets its own temporary profile, so parallel launches are safe.

## Generated Code

The [`chromiumoxide_pdl`](chromiumoxide_pdl) crate contains a [PDL parser](chromiumoxide_pdl/src/pdl/parser.rs), which is a rust rewrite of a [python script in the chromium source tree](https://chromium.googlesource.com/deps/inspector_protocol/+/refs/heads/master/pdl.py) and a [`Generator`](chromiumoxide_pdl/src/build/generator.rs) that turns the parsed PDL files into rust code. The [`chromiumoxide_cdp`](chromiumoxide_cdp) crate only purpose is to invoke the generator during its [build process](chromiumoxide_cdp/build.rs) and [include the generated output](chromiumoxide_cdp/src/lib.rs) before compiling the crate itself. This separation is done merely because the generated output is ~60K lines of rust code (not including all the proc macro expansions). So expect the compiling to take some time.
The generator can be configured and used independently, see [chromiumoxide_cdp/build.rs](chromiumoxide_cdp/build.rs).

Every chrome pdl domain is put in its own rust module, the types for the page domain of the browser_protocol are in `chromiumoxide_cdp::cdp::browser_protocol::page`, the runtime domain of the js_protocol in `chromiumoxide_cdp::cdp::js_protocol::runtime` and so on.

[vanilla.aslushnikov.com](https://vanilla.aslushnikov.com/) is a great resource to browse all the types defined in the pdl files. This site displays `Command` types as defined in the pdl files as `Method`. `chromiumoxid` sticks to the `Command` nomenclature. So for everything that is defined as a command type in the pdl (=marked as `Method` on [vanilla.aslushnikov.com](https://vanilla.aslushnikov.com/)) `chromiumoxide` contains a type for command and a designated type for the return type. For every command there is a `<name of command>Params` type with builder support (`<name of command>Params::builder()`) and its corresponding return type: `<name of command>Returns`. All commands share an implementation of the `chromiumoxide_types::Command` trait.
All Events are bundled in single enum (`CdpEvent`)

## Fetcher

By default `chromiumoxide` will try to find an installed version of chromium on the computer it runs on.
It is possible to download and install one automatically for some platforms using the `fetcher` feature.

You need to enable either the `rustls` or the `native-tls` feature and the `zip0` or `zip8` feature to allow the fetcher to download binaries.

```rust
use std::path::Path;

use chromiumoxide::fetcher::{BrowserFetcher, BrowserFetcherOptions};
use chromiumoxide::{Chromium, LaunchOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let download_path = Path::new("./download");
    tokio::fs::create_dir_all(&download_path).await?;
    let fetcher = BrowserFetcher::new(BrowserFetcherOptions::builder().with_path(&download_path).build()?);
    let info = fetcher.fetch().await?;
    let browser = Chromium::launch(LaunchOptions {
        executable_path: Some(info.executable_path),
        ..Default::default()
    })
    .await?;
    browser.close().await?;
    Ok(())
}
```

## License

Licensed under either of these:

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

## References

- [chromedp](https://github.com/chromedp/chromedp)
- [Playwright](https://github.com/microsoft/playwright) (Apache-2.0): behaviour, launch switches, device descriptors and screenshot scripts follow it.
- [rust-headless-chrome](https://github.com/atroche/rust-headless-chrome)
- [puppeteer](https://github.com/puppeteer/puppeteer)
