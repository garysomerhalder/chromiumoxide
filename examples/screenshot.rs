//! Renders HTML on an emulated iPhone and saves a full-page screenshot.
//!
//! `cargo run --example screenshot`

use chromiumoxide::{Chromium, LaunchOptions, device};

#[tokio::main]
async fn main() -> chromiumoxide::Result<()> {
    let browser = Chromium::launch(LaunchOptions::default()).await?;
    let iphone = device("iPhone 15").expect("known device");
    let context = browser.new_context(iphone.context_options()).await?;
    let page = context.new_page().await?;
    page.set_content("<h1 style='font-family:sans-serif'>Hello from chromiumoxide</h1>")
        .await?;
    page.screenshot()
        .full_page(true)
        .path("screenshot.png")
        .await?;
    println!("saved screenshot.png ({})", browser.version().await?);
    browser.close().await
}
