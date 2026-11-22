//! `toMatchAriaSnapshot`: passing cases checked in Playwright 1.63, failure
//! messages recorded from it.

use std::time::Duration;

use chromiumoxide::{Chromium, LaunchOptions, expect};

const HTML: &str =
    "<h1>Title</h1><ul><li>One</li><li><a href=\"/two\">Two</a></li></ul><button>OK</button>";

#[tokio::test]
async fn aria_snapshot_assertions() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(HTML).await.unwrap();
    let ul = page.locator("ul");

    expect(&page.locator("body"))
        .to_match_aria_snapshot(
            "
    - heading \"Title\" [level=1]
    - list:
      - listitem: One
  ",
        )
        .await
        .unwrap();
    expect(&page)
        .to_match_aria_snapshot("- button \"OK\"")
        .await
        .unwrap();
    expect(&ul)
        .to_match_aria_snapshot(
            "
    - list:
      - listitem: /O.e/
      - listitem:
        - link \"Two\":
          - /url: /two
  ",
        )
        .await
        .unwrap();
    expect(&ul)
        .not()
        .to_match_aria_snapshot("- list:\n  - listitem: Three")
        .await
        .unwrap();

    let err = expect(&ul)
        .to_match_aria_snapshot("\n    - list:\n      - listitem: Three\n  ")
        .timeout(Duration::from_secs(1))
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "expect(locator).toMatchAriaSnapshot(expected) failed\n\nLocator: locator('ul')\nTimeout: 1000ms\n- Expected  - 1\n+ Received  + 4\n\n  - list:\n-   - listitem: Three\n+   - listitem: One\n+   - listitem:\n+     - link \"Two\":\n+       - /url: /two"
    );

    let err = expect(&page)
        .to_match_aria_snapshot("- button \"Cancel\"")
        .timeout(Duration::from_secs(1))
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "expect(page).toMatchAriaSnapshot(expected) failed\n\nTimeout: 1000ms\n- Expected  - 1\n+ Received  + 7\n\n- - button \"Cancel\"\n+ - heading \"Title\" [level=1]\n+ - list:\n+   - listitem: One\n+   - listitem:\n+     - link \"Two\":\n+       - /url: /two\n+ - button \"OK\""
    );

    let err = expect(&ul)
        .to_match_aria_snapshot("- button [foo]")
        .timeout(Duration::from_secs(1))
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "expect(locator).toMatchAriaSnapshot(expected) failed\n\nLocator: locator('ul')\nExpected: \"- button [foo]\"\nError: Unsupported attribute [foo]:\n\nbutton [foo]\n        ^"
    );

    let err = expect(&ul)
        .not()
        .to_match_aria_snapshot("- listitem: One")
        .timeout(Duration::from_secs(1))
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "expect(locator).not.toMatchAriaSnapshot(expected) failed\n\nLocator:  locator('ul')\nExpected: not \"- listitem: One\"\nReceived: \"- list:\n  - listitem: One\n  - listitem:\n    - link \\\"Two\\\":\n      - /url: /two\"\nTimeout:  1000ms"
    );

    // A later change is waited for.
    page.evaluate::<()>(
        "() => { setTimeout(() => document.querySelector('button').textContent = 'Later', 300); }",
    )
    .await
    .unwrap();
    expect(&page.locator("button"))
        .to_match_aria_snapshot("- button \"Later\"")
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn aria_snapshot_files() {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(HTML).await.unwrap();
    let dir = std::env::temp_dir().join(format!("cox-aria-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = dir.join("list.aria.yml");
    let ul = page.locator("ul");

    let err = expect(&ul)
        .to_match_aria_snapshot_file(&file)
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "A snapshot doesn't exist at {}, writing actual.",
            file.display()
        )
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "- list:\n  - listitem: One\n  - listitem:\n    - link \"Two\":\n      - /url: /two"
    );
    expect(&ul)
        .to_match_aria_snapshot_file(&file)
        .await
        .unwrap();
    let _ = std::fs::remove_dir_all(dir);
    browser.close().await.unwrap();
}
