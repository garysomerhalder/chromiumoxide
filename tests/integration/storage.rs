//! Cookies, storage state and runtime context settings (expectations
//! recorded from Playwright 1.63).

use chromiumoxide::{
    Chromium, ClearCookies, ContextOptions, Cookie, Geolocation, LaunchOptions, NewCookie, SameSite,
};
use serde_json::{Value, json};

use crate::server::TestServer;

fn names(cookies: &[Cookie]) -> Vec<String> {
    let mut n: Vec<String> = cookies.iter().map(|c| c.name.clone()).collect();
    n.sort();
    n
}

#[tokio::test]
async fn cookies_and_storage_state_like_playwright() {
    let server = TestServer::start().await;
    server.respond(
        "/setcookie",
        "200 OK",
        &[
            ("Content-Type", "text/html"),
            ("Set-Cookie", "srv=1; HttpOnly; Path=/"),
            ("Set-Cookie", "strict=2; SameSite=Strict"),
        ],
        "ok",
    );
    server.route("/", "text/html", "<p>hi</p>", 0);
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let context = browser
        .new_context(ContextOptions::default())
        .await
        .unwrap();
    let page = context.new_page().await.unwrap();
    context
        .add_cookies(&[
            NewCookie::new("added", "v").url(&server.url("/")),
            NewCookie::new("dom", "d")
                .domain("127.0.0.1")
                .path("/sub")
                .expires(2_000_000_000.0)
                .same_site(SameSite::Strict),
        ])
        .await
        .unwrap();
    page.goto(&server.url("/setcookie")).await.unwrap();
    page.evaluate::<()>(
        "() => { document.cookie = 'js=3'; localStorage.setItem('k', 'v'); localStorage.setItem('k2', 'v2'); }",
    )
    .await
    .unwrap();

    let mut cookies = context.cookies().await.unwrap();
    cookies.sort_by(|a, b| a.name.cmp(&b.name));
    // Chromium caps cookie lifetimes at 400 days (Playwright reports the same).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let dom = cookies.iter().find(|c| c.name == "dom").unwrap();
    assert!(
        (dom.expires - (now + 400.0 * 86400.0)).abs() < 60.0,
        "{}",
        dom.expires
    );
    let as_json: Vec<Value> = cookies
        .iter()
        .map(|c| {
            let mut v = serde_json::to_value(c).unwrap();
            if c.name == "dom" {
                v["expires"] = json!("capped");
            }
            v
        })
        .collect();
    assert_eq!(
        as_json,
        [
            json!({"name":"added","value":"v","domain":"127.0.0.1","path":"/","expires":-1.0,"httpOnly":false,"secure":false,"sameSite":"Lax"}),
            json!({"name":"dom","value":"d","domain":"127.0.0.1","path":"/sub","expires":"capped","httpOnly":false,"secure":false,"sameSite":"Strict"}),
            json!({"name":"js","value":"3","domain":"127.0.0.1","path":"/","expires":-1.0,"httpOnly":false,"secure":false,"sameSite":"Lax"}),
            json!({"name":"srv","value":"1","domain":"127.0.0.1","path":"/","expires":-1.0,"httpOnly":true,"secure":false,"sameSite":"Lax"}),
            json!({"name":"strict","value":"2","domain":"127.0.0.1","path":"/","expires":-1.0,"httpOnly":false,"secure":false,"sameSite":"Strict"}),
        ]
    );
    assert_eq!(
        names(&context.cookies_for(&[&server.url("/")]).await.unwrap()),
        ["added", "js", "srv", "strict"]
    );
    assert_eq!(
        names(&context.cookies_for(&[&server.url("/sub/x")]).await.unwrap()),
        ["added", "dom", "js", "srv", "strict"]
    );
    assert!(
        context
            .cookies_for(&["http://example.com"])
            .await
            .unwrap()
            .is_empty()
    );

    let state = context.storage_state().await.unwrap();
    assert_eq!(names(&state.cookies), names(&cookies));
    assert_eq!(state.origins.len(), 1);
    let origin = server.url("");
    assert_eq!(state.origins[0].origin, origin);
    let mut ls: Vec<(String, String)> = state.origins[0]
        .local_storage
        .iter()
        .map(|n| (n.name.clone(), n.value.clone()))
        .collect();
    ls.sort();
    assert_eq!(ls, [("k".into(), "v".into()), ("k2".into(), "v2".into())]);

    context
        .clear_cookies_matching(ClearCookies::default().name("js"))
        .await
        .unwrap();
    assert_eq!(
        names(&context.cookies().await.unwrap()),
        ["added", "dom", "srv", "strict"]
    );

    // Runtime settings.
    context
        .set_extra_http_headers([("X-Foo", "bar")])
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<Value>("() => fetch('/headers').then(r => r.json())")
            .await
            .unwrap(),
        json!({"foo": "bar", "cookie": "added=v; srv=1; strict=2"})
    );
    context.set_offline(true).await.unwrap();
    assert_eq!(
        page.evaluate::<Value>(
            "async () => [await fetch('/headers').then(() => 'ok', e => e.message), navigator.onLine]"
        )
        .await
        .unwrap(),
        json!(["Failed to fetch", false])
    );
    context.set_offline(false).await.unwrap();
    context
        .grant_permissions(&["geolocation"], None)
        .await
        .unwrap();
    context
        .set_geolocation(Some(Geolocation {
            latitude: 10.0,
            longitude: 20.0,
            accuracy: 0.0,
        }))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<Value>(
            "() => new Promise(r => navigator.geolocation.getCurrentPosition(p => r([p.coords.latitude, p.coords.longitude, p.coords.accuracy]), e => r(e.message)))"
        )
        .await
        .unwrap(),
        json!([10, 20, 0])
    );
    let perm = "() => navigator.permissions.query({name: 'geolocation'}).then(r => r.state)";
    assert_eq!(page.evaluate::<String>(perm).await.unwrap(), "granted");
    context.clear_permissions().await.unwrap();
    assert_eq!(page.evaluate::<String>(perm).await.unwrap(), "prompt");
    assert!(
        context
            .grant_permissions(&["bogus"], None)
            .await
            .unwrap_err()
            .to_string()
            .contains("Unknown permission: bogus")
    );
    context.clear_cookies().await.unwrap();
    assert!(context.cookies().await.unwrap().is_empty());

    // Restoring a saved state into a new context.
    let c2 = browser
        .new_context(ContextOptions {
            storage_state: Some(state),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(c2.pages().is_empty(), "the helper page is closed");
    let p2 = c2.new_page().await.unwrap();
    p2.goto(&server.url("/")).await.unwrap();
    let (item, cookie): (Option<String>, String) = p2
        .evaluate("() => [localStorage.getItem('k'), document.cookie]")
        .await
        .unwrap();
    assert_eq!(item.as_deref(), Some("v"));
    let mut sent: Vec<&str> = cookie.split("; ").collect();
    sent.sort();
    assert_eq!(sent, ["added=v", "js=3", "strict=2"]);
    assert_eq!(c2.cookies().await.unwrap().len(), 5);
    // Origins visited are kept, even without a page on them.
    p2.close().await.unwrap();
    let state2 = c2.storage_state().await.unwrap();
    assert_eq!(state2.origins.len(), 1);
    assert_eq!(state2.origins[0].origin, origin);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn page_extra_http_headers() {
    let server = TestServer::start().await;
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_extra_http_headers([("x-foo", "page")])
        .await
        .unwrap();
    page.goto(&server.url("/headers")).await.unwrap();
    assert_eq!(
        page.evaluate::<Value>("() => JSON.parse(document.body.innerText)")
            .await
            .unwrap(),
        json!({"foo": "page", "cookie": null})
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn storage_state_with_indexed_db_round_trips() {
    let server = crate::server::TestServer::start().await;
    server.route("/", "text/html", "<p>x</p>", 0);
    let browser = chromiumoxide::Chromium::launch(chromiumoxide::LaunchOptions::default())
        .await
        .unwrap();
    let ctx = browser.new_context(Default::default()).await.unwrap();
    let page = ctx.new_page().await.unwrap();
    page.goto(&server.url("/")).await.unwrap();
    page.evaluate::<()>(r#"async () => {
    localStorage.setItem('k', 'v');
    await new Promise((res, rej) => {
      const r = indexedDB.open('db1', 2);
      r.onupgradeneeded = () => { const s = r.result.createObjectStore('items', { keyPath: 'id' }); s.createIndex('byName', 'name', { unique: false }); r.result.createObjectStore('free', { autoIncrement: true }); };
      r.onsuccess = () => { const t = r.result.transaction(['items', 'free'], 'readwrite'); t.objectStore('items').put({ id: 1, name: 'a', when: new Date(0) }); t.objectStore('free').put('plain'); t.oncomplete = () => { r.result.close(); res(); }; };
      r.onerror = rej;
    });
  }"#)
    .await
    .unwrap();
    let state = ctx
        .storage_state_with(chromiumoxide::StorageStateOptions {
            indexed_db: true,
            ..Default::default()
        })
        .await
        .unwrap();
    // Recorded from Playwright 1.63 (storageState({ indexedDB: true })).
    assert_eq!(
        serde_json::to_value(&state.origins).unwrap(),
        serde_json::json!([{"origin": server.url(""), "localStorage": [{"name": "k", "value": "v"}],
            "indexedDB": [{"name": "db1", "version": 2, "stores": [
                {"name": "free", "autoIncrement": true, "records": [{"key": 1, "value": "plain"}], "indexes": []},
                {"name": "items", "autoIncrement": false, "keyPath": "id",
                 "records": [{"valueEncoded": {"o": [{"k": "id", "v": 1}, {"k": "name", "v": "a"}, {"k": "when", "v": {"d": "1970-01-01T00:00:00.000Z"}}], "id": 1}}],
                 "indexes": [{"name": "byName", "keyPath": "name", "multiEntry": false, "unique": false}]}]}]}])
    );
    // Without the option, IndexedDB is left out (like Playwright).
    let plain = ctx.storage_state().await.unwrap();
    assert!(
        serde_json::to_value(&plain.origins).unwrap()[0]
            .get("indexedDB")
            .is_none()
    );

    let ctx2 = browser
        .new_context(chromiumoxide::ContextOptions {
            storage_state: Some(state),
            ..Default::default()
        })
        .await
        .unwrap();
    let p2 = ctx2.new_page().await.unwrap();
    p2.goto(&server.url("/")).await.unwrap();
    let got: serde_json::Value = p2
        .evaluate(r#"async () => { const db = await new Promise(r => { const o = indexedDB.open('db1'); o.onsuccess = () => r(o.result); }); const v = await new Promise(r => { const q = db.transaction('items').objectStore('items').get(1); q.onsuccess = () => r(q.result); }); return JSON.parse(JSON.stringify({ v, isDate: v.when instanceof Date, ls: localStorage.getItem('k'), version: db.version })); }"#)
        .await
        .unwrap();
    assert_eq!(
        got,
        serde_json::json!({"v": {"id": 1, "name": "a", "when": "1970-01-01T00:00:00.000Z"}, "isDate": true, "ls": "v", "version": 2})
    );
    browser.close().await.unwrap();
}
