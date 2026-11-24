// The test app: one window showing dist/index.html.
//
// Automation opt-in on Windows: Tauri sets its own WebView2 arguments, and
// WebView2 then ignores WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, so the app
// passes that variable on itself (chromiumoxide sets it to open a DevTools
// port). On Linux, Tauri's own opt-in is TAURI_WEBVIEW_AUTOMATION=true.
// On macOS the app embeds a WebDriver server (tauri-plugin-wdio-webdriver)
// when chromiumoxide sets TAURI_WEBDRIVER_PORT.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    if std::env::var_os("TAURI_WEBDRIVER_PORT").is_some() {
        builder = builder.plugin(tauri_plugin_wdio_webdriver::init());
    }
    builder
        .setup(|app| {
            #[allow(unused_mut)]
            let mut window =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                    .title("Cox Tauri")
                    .inner_size(800.0, 600.0);
            #[cfg(windows)]
            if let Ok(extra) = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
                window = window.additional_browser_args(&format!(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection {extra}"
                ));
            }
            window.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the test app");
}
