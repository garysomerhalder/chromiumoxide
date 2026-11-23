//! One test per CDP domain, through the generated typed API
//! (`page.protocol().<domain>().<command>(..)` and `.on_<event>()`), against
//! real Chromium.
//!
//! Domains that need hardware, accounts or a DevTools front end the container
//! lacks (Cast sinks, smart cards, Bluetooth radios, installed web apps, a
//! wallet) are checked with [`answered`]: Chromium knows the command and
//! answers it, either with a result or with a protocol error about the missing
//! device.

use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::{
    accessibility, ads, animation, audits, autofill, background_service, bluetooth_emulation,
    browser, cache_storage, cast, crash_report_context, css, device_access, device_orientation,
    digital_credentials, dom, dom_debugger, dom_snapshot, dom_storage, emulation,
    event_breakpoints, extensions, fed_cm, fetch, file_system, find_in_page, headless_experimental,
    indexed_db, input, inspector, io, layer_tree, log, media, memory, network, overlay, page,
    performance, performance_timeline, preload, pwa, security, service_worker,
    smart_card_emulation, storage, system_info, target, tethering, tracing, web_audio, web_authn,
    web_mcp,
};
use chromiumoxide::cdp::js_protocol::{debugger, heap_profiler, profiler, runtime};
use chromiumoxide::{Browser, Chromium, Error, LaunchOptions, Page};
use futures::StreamExt;

use crate::server::TestServer;

async fn page() -> (Browser, Page) {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    (browser, page)
}

/// Chromium knows the command: it answered with a result or with a protocol
/// error other than "method not found" (-32601).
fn answered<T: std::fmt::Debug>(result: Result<T, Error>) {
    match result {
        Ok(_) => {}
        Err(Error::Protocol { code, message, .. }) => {
            assert_ne!(
                code, -32601,
                "Chromium does not know the command: {message}"
            )
        }
        Err(e) => panic!("not a protocol answer: {e}"),
    }
}

async fn next<S: futures::Stream + Unpin>(stream: &mut S) -> S::Item {
    tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .expect("event within 10s")
        .expect("stream open")
}

#[tokio::test]
async fn cdp_accessibility() {
    let (browser, page) = page().await;
    page.set_content("<button>Go</button>").await.unwrap();
    let tree = page
        .protocol()
        .accessibility()
        .get_full_ax_tree(accessibility::GetFullAxTreeParams::default())
        .await
        .unwrap();
    assert!(!tree.nodes.is_empty());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_ads() {
    let (browser, page) = page().await;
    answered(
        page.protocol()
            .ads()
            .get_ad_scripts(ads::GetAdScriptsParams::default())
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_animation() {
    let (browser, page) = page().await;
    let animation = page.protocol().animation();
    animation
        .enable(animation::EnableParams::default())
        .await
        .unwrap();
    animation
        .set_playback_rate(animation::SetPlaybackRateParams::new(0.5))
        .await
        .unwrap();
    let rate = animation
        .get_playback_rate(animation::GetPlaybackRateParams::default())
        .await
        .unwrap();
    assert_eq!(rate.playback_rate, 0.5);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_audits() {
    let (browser, page) = page().await;
    page.set_content("<form><input id=a><input id=a></form>")
        .await
        .unwrap();
    let audits = page.protocol().audits();
    audits
        .enable(audits::EnableParams::default())
        .await
        .unwrap();
    audits
        .check_forms_issues(audits::CheckFormsIssuesParams::default())
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_autofill() {
    let (browser, page) = page().await;
    answered(
        page.protocol()
            .autofill()
            .set_addresses(autofill::SetAddressesParams::new(vec![]))
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_background_service() {
    let (browser, page) = page().await;
    let service = page.protocol().background_service();
    let mut states = service.on_recording_state_changed();
    service
        .set_recording(background_service::SetRecordingParams::new(
            true,
            background_service::ServiceName::BackgroundFetch,
        ))
        .await
        .unwrap();
    service
        .start_observing(background_service::StartObservingParams::new(
            background_service::ServiceName::BackgroundFetch,
        ))
        .await
        .unwrap();
    let state = next(&mut states).await;
    assert!(state.is_recording);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_bluetooth_emulation() {
    let (browser, page) = page().await;
    answered(
        page.protocol()
            .bluetooth_emulation()
            .enable(bluetooth_emulation::EnableParams::new(
                bluetooth_emulation::CentralState::PoweredOn,
                true,
            ))
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_browser() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let version = browser
        .protocol()
        .browser()
        .get_version(browser::GetVersionParams::default())
        .await
        .unwrap();
    assert!(version.product.contains("Chrome"), "{}", version.product);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_cache_storage() {
    let (browser, page) = page().await;
    answered(
        page.protocol()
            .cache_storage()
            .request_cache_names(
                cache_storage::RequestCacheNamesParams::builder()
                    .security_origin("https://example.com")
                    .build(),
            )
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_cast() {
    let (browser, page) = page().await;
    answered(
        page.protocol()
            .cast()
            .enable(cast::EnableParams::default())
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_crash_report_context() {
    let (browser, page) = page().await;
    answered(
        page.protocol()
            .crash_report_context()
            .get_entries(crash_report_context::GetEntriesParams::default())
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_css() {
    let (browser, page) = page().await;
    page.set_content("<style>@media (min-width: 1px) { p { color: red } }</style><p>x</p>")
        .await
        .unwrap();
    let cdp = page.protocol();
    cdp.dom()
        .enable(dom::EnableParams::default())
        .await
        .unwrap();
    cdp.css()
        .enable(css::EnableParams::default())
        .await
        .unwrap();
    let queries = cdp
        .css()
        .get_media_queries(css::GetMediaQueriesParams::default())
        .await
        .unwrap();
    assert!(!queries.medias.is_empty());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_device_access() {
    let (browser, page) = page().await;
    let access = page.protocol().device_access();
    access
        .enable(device_access::EnableParams::default())
        .await
        .unwrap();
    access
        .disable(device_access::DisableParams::default())
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_device_orientation() {
    let (browser, page) = page().await;
    let orientation = page.protocol().device_orientation();
    orientation
        .set_device_orientation_override(
            device_orientation::SetDeviceOrientationOverrideParams::new(10.0, 20.0, 30.0),
        )
        .await
        .unwrap();
    orientation
        .clear_device_orientation_override(
            device_orientation::ClearDeviceOrientationOverrideParams::default(),
        )
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_digital_credentials() {
    let (browser, page) = page().await;
    answered(
        page.protocol()
            .digital_credentials()
            .set_virtual_wallet_behavior(digital_credentials::SetVirtualWalletBehaviorParams::new(
                digital_credentials::VirtualWalletAction::Decline,
            ))
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_dom() {
    let (browser, page) = page().await;
    page.set_content("<p id=x>x</p>").await.unwrap();
    let dom = page.protocol().dom();
    let doc = dom
        .get_document(dom::GetDocumentParams::default())
        .await
        .unwrap();
    assert_eq!(doc.root.node_name, "#document");
    let found = dom
        .query_selector(dom::QuerySelectorParams::new(doc.root.node_id, "#x"))
        .await
        .unwrap();
    let html = dom
        .get_outer_html(
            dom::GetOuterHtmlParams::builder()
                .node_id(found.node_id)
                .build(),
        )
        .await
        .unwrap();
    assert_eq!(html.outer_html, r#"<p id="x">x</p>"#);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_dom_debugger() {
    let (browser, page) = page().await;
    page.set_content("<p>x</p><script>document.addEventListener('click', () => {})</script>")
        .await
        .unwrap();
    let cdp = page.protocol();
    let doc = cdp
        .runtime()
        .evaluate(runtime::EvaluateParams::new("document"))
        .await
        .unwrap();
    let listeners = cdp
        .dom_debugger()
        .get_event_listeners(dom_debugger::GetEventListenersParams::new(
            doc.result.object_id.unwrap(),
        ))
        .await
        .unwrap();
    assert!(listeners.listeners.iter().any(|l| l.r#type == "click"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_dom_snapshot() {
    let (browser, page) = page().await;
    page.set_content("<p>x</p>").await.unwrap();
    let snapshot = page
        .protocol()
        .dom_snapshot()
        .capture_snapshot(dom_snapshot::CaptureSnapshotParams::new(vec![]))
        .await
        .unwrap();
    assert!(!snapshot.documents.is_empty());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_dom_storage() {
    let server = TestServer::start().await;
    server.route("/storage.html", "text/html", "<p>x</p>", 0);
    let (browser, page) = page().await;
    page.goto(&server.url("/storage.html")).await.unwrap();
    page.evaluate::<()>("localStorage.setItem('k', 'v')")
        .await
        .unwrap();
    let cdp = page.protocol();
    let key = cdp
        .storage()
        .get_storage_key(storage::GetStorageKeyParams::default())
        .await
        .unwrap();
    let items = cdp
        .dom_storage()
        .get_dom_storage_items(dom_storage::GetDomStorageItemsParams::new(
            dom_storage::StorageId::builder()
                .storage_key(key.storage_key.inner().clone())
                .is_local_storage(true)
                .build()
                .unwrap(),
        ))
        .await
        .unwrap();
    assert!(
        items
            .entries
            .iter()
            .any(|e| e.inner() == &vec!["k".to_string(), "v".to_string()])
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_emulation() {
    let (browser, page) = page().await;
    page.protocol()
        .emulation()
        .set_timezone_override(emulation::SetTimezoneOverrideParams::new("Asia/Tokyo"))
        .await
        .unwrap();
    let tz: String = page
        .evaluate("Intl.DateTimeFormat().resolvedOptions().timeZone")
        .await
        .unwrap();
    assert_eq!(tz, "Asia/Tokyo");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_event_breakpoints() {
    let (browser, page) = page().await;
    let breakpoints = page.protocol().event_breakpoints();
    breakpoints
        .set_instrumentation_breakpoint(event_breakpoints::SetInstrumentationBreakpointParams::new(
            "setTimeout",
        ))
        .await
        .unwrap();
    breakpoints
        .remove_instrumentation_breakpoint(
            event_breakpoints::RemoveInstrumentationBreakpointParams::new("setTimeout"),
        )
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_extensions() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    answered(
        browser
            .protocol()
            .extensions()
            .get_extensions(extensions::GetExtensionsParams::default())
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_fed_cm() {
    let (browser, page) = page().await;
    let fed_cm = page.protocol().fed_cm();
    fed_cm
        .enable(fed_cm::EnableParams::default())
        .await
        .unwrap();
    fed_cm
        .reset_cooldown(fed_cm::ResetCooldownParams::default())
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_fetch() {
    let server = TestServer::start().await;
    server.route("/fetch.html", "text/html", "<p>x</p>", 0);
    let (browser, page) = page().await;
    let fetch = page.protocol().fetch();
    let mut paused = fetch.on_request_paused();
    fetch.enable(fetch::EnableParams::default()).await.unwrap();
    let goto = {
        let page = page.clone();
        let url = server.url("/fetch.html");
        tokio::spawn(async move { page.goto(&url).await })
    };
    let request = next(&mut paused).await;
    assert!(request.request.url.ends_with("/fetch.html"));
    fetch
        .continue_request(fetch::ContinueRequestParams::new(
            request.request_id.clone(),
        ))
        .await
        .unwrap();
    goto.await.unwrap().unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_file_system() {
    let server = TestServer::start().await;
    server.route("/fs.html", "text/html", "<p>x</p>", 0);
    let (browser, page) = page().await;
    page.goto(&server.url("/fs.html")).await.unwrap();
    let cdp = page.protocol();
    let key = cdp
        .storage()
        .get_storage_key(storage::GetStorageKeyParams::default())
        .await
        .unwrap();
    answered(
        cdp.file_system()
            .get_directory(file_system::GetDirectoryParams::new(
                file_system::BucketFileSystemLocator::builder()
                    .storage_key(key.storage_key)
                    .path_components(Vec::<String>::new())
                    .build()
                    .unwrap(),
            ))
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_find_in_page() {
    let (browser, page) = page().await;
    page.set_content("<p>needle</p>").await.unwrap();
    answered(
        page.protocol()
            .find_in_page()
            .find_first(find_in_page::FindFirstParams::new("needle"))
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_headless_experimental() {
    let (browser, page) = page().await;
    answered(
        page.protocol()
            .headless_experimental()
            .begin_frame(headless_experimental::BeginFrameParams::default())
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_indexed_db() {
    let server = TestServer::start().await;
    server.route("/idb.html", "text/html", "<p>x</p>", 0);
    let (browser, page) = page().await;
    page.goto(&server.url("/idb.html")).await.unwrap();
    page.evaluate::<()>(
        "new Promise(r => { const q = indexedDB.open('cox'); q.onsuccess = () => r(); })",
    )
    .await
    .unwrap();
    let names = page
        .protocol()
        .indexed_db()
        .request_database_names(
            indexed_db::RequestDatabaseNamesParams::builder()
                .security_origin(server.url("").trim_end_matches('/'))
                .build(),
        )
        .await
        .unwrap();
    assert!(names.database_names.contains(&"cox".to_string()));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_input() {
    let (browser, page) = page().await;
    page.set_content("<input autofocus>").await.unwrap();
    page.locator("input").focus().await.unwrap();
    page.protocol()
        .input()
        .insert_text(input::InsertTextParams::new("typed"))
        .await
        .unwrap();
    let value: String = page
        .evaluate("document.querySelector('input').value")
        .await
        .unwrap();
    assert_eq!(value, "typed");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_inspector() {
    let (browser, page) = page().await;
    let inspector = page.protocol().inspector();
    inspector
        .enable(inspector::EnableParams::default())
        .await
        .unwrap();
    inspector
        .disable(inspector::DisableParams::default())
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_io() {
    let (browser, page) = page().await;
    page.set_content("<p>pdf</p>").await.unwrap();
    let cdp = page.protocol();
    let pdf = cdp
        .page()
        .print_to_pdf(
            page::PrintToPdfParams::builder()
                .transfer_mode(page::PrintToPdfTransferMode::ReturnAsStream)
                .build(),
        )
        .await
        .unwrap();
    let handle = pdf.stream.unwrap();
    let chunk = cdp
        .io()
        .read(io::ReadParams::new(handle.clone()))
        .await
        .unwrap();
    assert!(!chunk.data.is_empty());
    cdp.io().close(io::CloseParams::new(handle)).await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_layer_tree() {
    let (browser, page) = page().await;
    let layers = page.protocol().layer_tree();
    layers
        .enable(layer_tree::EnableParams::default())
        .await
        .unwrap();
    layers
        .disable(layer_tree::DisableParams::default())
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_log() {
    let (browser, page) = page().await;
    let log = page.protocol().log();
    let mut entries = log.on_entry_added();
    log.enable(log::EnableParams::default()).await.unwrap();
    log.start_violations_report(log::StartViolationsReportParams::new(vec![
        log::ViolationSetting::new(log::ViolationSettingName::DiscouragedApiUse, 0.0),
    ]))
    .await
    .unwrap();
    page.evaluate::<()>("document.write('<p>x</p>')")
        .await
        .unwrap();
    let entry = next(&mut entries).await;
    assert!(!entry.entry.text.is_empty());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_media() {
    let (browser, page) = page().await;
    let media = page.protocol().media();
    media.enable(media::EnableParams::default()).await.unwrap();
    media
        .disable(media::DisableParams::default())
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_memory() {
    let (browser, page) = page().await;
    let counters = page
        .protocol()
        .memory()
        .get_dom_counters(memory::GetDomCountersParams::default())
        .await
        .unwrap();
    assert!(counters.documents >= 1);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_network() {
    let server = TestServer::start().await;
    server.route("/net.html", "text/html", "<p>x</p>", 0);
    let (browser, page) = page().await;
    let network = page.protocol().network();
    let mut requests = network.on_request_will_be_sent();
    network
        .enable(network::EnableParams::default())
        .await
        .unwrap();
    page.goto(&server.url("/net.html")).await.unwrap();
    let request = next(&mut requests).await;
    assert!(request.request.url.ends_with("/net.html"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_overlay() {
    let (browser, page) = page().await;
    let cdp = page.protocol();
    cdp.dom()
        .enable(dom::EnableParams::default())
        .await
        .unwrap();
    cdp.overlay()
        .enable(overlay::EnableParams::default())
        .await
        .unwrap();
    cdp.overlay()
        .set_show_fps_counter(overlay::SetShowFpsCounterParams::new(true))
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_page() {
    let (browser, page) = page().await;
    let cdp = page.protocol().page();
    let mut loads = cdp.on_load_event_fired();
    cdp.enable(page::EnableParams::default()).await.unwrap();
    cdp.navigate(page::NavigateParams::new("data:text/html,<p>x</p>"))
        .await
        .unwrap();
    next(&mut loads).await;
    let tree = cdp
        .get_frame_tree(page::GetFrameTreeParams::default())
        .await
        .unwrap();
    assert!(tree.frame_tree.frame.url.starts_with("data:"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_performance() {
    let (browser, page) = page().await;
    let performance = page.protocol().performance();
    performance
        .enable(performance::EnableParams::default())
        .await
        .unwrap();
    let metrics = performance
        .get_metrics(performance::GetMetricsParams::default())
        .await
        .unwrap();
    assert!(metrics.metrics.iter().any(|m| m.name == "Nodes"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_performance_timeline() {
    let (browser, page) = page().await;
    page.protocol()
        .performance_timeline()
        .enable(performance_timeline::EnableParams::new(vec![
            "largest-contentful-paint".to_string(),
        ]))
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_preload() {
    let (browser, page) = page().await;
    let preload = page.protocol().preload();
    preload
        .enable(preload::EnableParams::default())
        .await
        .unwrap();
    preload
        .disable(preload::DisableParams::default())
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_pwa() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    answered(
        browser
            .protocol()
            .pwa()
            .get_os_app_state(pwa::GetOsAppStateParams::new("https://example.com/app"))
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_security() {
    let (browser, page) = page().await;
    let security = page.protocol().security();
    let mut states = security.on_visible_security_state_changed();
    security
        .enable(security::EnableParams::default())
        .await
        .unwrap();
    page.goto("data:text/html,<p>x</p>").await.unwrap();
    next(&mut states).await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_service_worker() {
    let (browser, page) = page().await;
    let workers = page.protocol().service_worker();
    workers
        .enable(service_worker::EnableParams::default())
        .await
        .unwrap();
    workers
        .stop_all_workers(service_worker::StopAllWorkersParams::default())
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_smart_card_emulation() {
    let (browser, page) = page().await;
    answered(
        page.protocol()
            .smart_card_emulation()
            .enable(smart_card_emulation::EnableParams::default())
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_storage() {
    let server = TestServer::start().await;
    server.route("/quota.html", "text/html", "<p>x</p>", 0);
    let (browser, page) = page().await;
    page.goto(&server.url("/quota.html")).await.unwrap();
    let usage = page
        .protocol()
        .storage()
        .get_usage_and_quota(storage::GetUsageAndQuotaParams::new(
            server.url("").trim_end_matches('/'),
        ))
        .await
        .unwrap();
    assert!(usage.quota > 0.0);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_system_info() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let info = browser
        .protocol()
        .system_info()
        .get_process_info(system_info::GetProcessInfoParams::default())
        .await
        .unwrap();
    assert!(info.process_info.iter().any(|p| p.r#type == "browser"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_target() {
    let (browser, _page) = page().await;
    let targets = browser
        .protocol()
        .target()
        .get_targets(target::GetTargetsParams::default())
        .await
        .unwrap();
    assert!(targets.target_infos.iter().any(|t| t.r#type == "page"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_tethering() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    answered(
        browser
            .protocol()
            .tethering()
            .unbind(tethering::UnbindParams::new(0))
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_tracing() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let categories = browser
        .protocol()
        .tracing()
        .get_categories(tracing::GetCategoriesParams::default())
        .await
        .unwrap();
    assert!(
        categories
            .categories
            .iter()
            .any(|c| c == "devtools.timeline")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_web_audio() {
    let (browser, page) = page().await;
    let audio = page.protocol().web_audio();
    let mut created = audio.on_context_created();
    audio
        .enable(web_audio::EnableParams::default())
        .await
        .unwrap();
    page.evaluate::<()>("window.ctx = new AudioContext()")
        .await
        .unwrap();
    let context = next(&mut created).await;
    assert!(!context.context.context_id.inner().is_empty());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_web_authn() {
    let (browser, page) = page().await;
    let authn = page.protocol().web_authn();
    authn
        .enable(web_authn::EnableParams::default())
        .await
        .unwrap();
    let authenticator = authn
        .add_virtual_authenticator(web_authn::AddVirtualAuthenticatorParams::new(
            web_authn::VirtualAuthenticatorOptions::builder()
                .protocol(web_authn::AuthenticatorProtocol::Ctap2)
                .transport(web_authn::AuthenticatorTransport::Usb)
                .build()
                .unwrap(),
        ))
        .await
        .unwrap();
    let credentials = authn
        .get_credentials(web_authn::GetCredentialsParams::new(
            authenticator.authenticator_id,
        ))
        .await
        .unwrap();
    assert!(credentials.credentials.is_empty());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_web_mcp() {
    let (browser, page) = page().await;
    answered(
        page.protocol()
            .web_mcp()
            .enable(web_mcp::EnableParams::default())
            .await,
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_debugger() {
    let (browser, page) = page().await;
    let debugger = page.protocol().debugger();
    let mut parsed = debugger.on_script_parsed();
    debugger
        .enable(debugger::EnableParams::default())
        .await
        .unwrap();
    page.evaluate::<i64>("40 + 2\n//# sourceURL=cox-debugger.js")
        .await
        .unwrap();
    let script = loop {
        let script = next(&mut parsed).await;
        if script.url == "cox-debugger.js" {
            break script;
        }
    };
    let source = debugger
        .get_script_source(debugger::GetScriptSourceParams::new(script.script_id))
        .await
        .unwrap();
    assert!(source.script_source.starts_with("40 + 2"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_heap_profiler() {
    let (browser, page) = page().await;
    let heap = page.protocol().heap_profiler();
    heap.enable(heap_profiler::EnableParams::default())
        .await
        .unwrap();
    heap.collect_garbage(heap_profiler::CollectGarbageParams::default())
        .await
        .unwrap();
    heap.start_sampling(heap_profiler::StartSamplingParams::default())
        .await
        .unwrap();
    let profile = heap
        .stop_sampling(heap_profiler::StopSamplingParams::default())
        .await
        .unwrap();
    assert!(profile.profile.head.call_frame.function_name.is_empty());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_profiler() {
    let (browser, page) = page().await;
    let profiler = page.protocol().profiler();
    profiler
        .enable(profiler::EnableParams::default())
        .await
        .unwrap();
    profiler
        .start(profiler::StartParams::default())
        .await
        .unwrap();
    page.evaluate::<i64>("let s = 0; for (let i = 0; i < 1e5; i++) s += i; s")
        .await
        .unwrap();
    let profile = profiler
        .stop(profiler::StopParams::default())
        .await
        .unwrap();
    assert!(!profile.profile.nodes.is_empty());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cdp_runtime() {
    let (browser, page) = page().await;
    let runtime = page.protocol().runtime();
    let mut console = runtime.on_console_api_called();
    runtime
        .enable(runtime::EnableParams::default())
        .await
        .unwrap();
    let r = runtime
        .evaluate(runtime::EvaluateParams::new("console.log('hi'); 6 * 7"))
        .await
        .unwrap();
    assert_eq!(r.result.value, Some(serde_json::json!(42)));
    let call = next(&mut console).await;
    assert_eq!(call.args[0].value, Some(serde_json::json!("hi")));
    browser.close().await.unwrap();
}
