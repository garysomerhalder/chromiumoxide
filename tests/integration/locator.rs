use std::time::Duration;

use chromiumoxide::{
    Browser, Chromium, Error, Filter, LaunchOptions, Page, RoleOptions, TextMatch, WaitForState,
};

async fn page_with(html: &str) -> (Browser, Page) {
    let browser = Chromium::launch(LaunchOptions::default()).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.set_content(html).await.unwrap();
    (browser, page)
}

const FORM: &str = r#"
<h1>Sign up</h1>
<label for=email>Email</label><input id=email placeholder="you@example.com">
<label><input type=checkbox id=terms> Accept terms</label>
<select id=color><option value=r>Red</option><option value=g>Green</option><option value=b>Blue</option></select>
<input type=file id=upload>
<img alt="Logo" src="data:image/gif;base64,R0lGODlhAQABAAAAACw=" title="Company">
<button data-testid=submit onclick="window.submitted = (window.submitted||0) + 1">Submit</button>
<button disabled>Disabled</button>
<ul><li>One</li><li>Two <b>bold</b></li><li>Three</li></ul>
<div id=hover-target onmouseover="window.hovered = true" style="width:50px;height:20px">hover</div>
"#;

#[tokio::test]
async fn selectors_and_get_by_engines_find_elements() {
    let (browser, page) = page_with(FORM).await;
    assert_eq!(
        page.get_by_role("heading", RoleOptions::default())
            .text_content()
            .await
            .unwrap()
            .as_deref(),
        Some("Sign up")
    );
    let submit = RoleOptions {
        name: Some("Submit".into()),
        ..Default::default()
    };
    assert_eq!(page.get_by_role("button", submit).count().await.unwrap(), 1);
    assert_eq!(
        page.get_by_role("button", RoleOptions::default())
            .count()
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        page.get_by_label("Email", false)
            .get_attribute("id")
            .await
            .unwrap()
            .as_deref(),
        Some("email")
    );
    assert_eq!(
        page.get_by_placeholder("you@example", false)
            .count()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        page.get_by_test_id("submit").inner_text().await.unwrap(),
        "Submit"
    );
    assert_eq!(
        page.get_by_alt_text("logo", false).count().await.unwrap(),
        1
    );
    assert_eq!(page.get_by_title("Company", true).count().await.unwrap(), 1);
    assert_eq!(page.get_by_text("Accept", false).count().await.unwrap(), 1);
    assert_eq!(
        page.get_by_text(TextMatch::regex("^two", "i"), false)
            .count()
            .await
            .unwrap(),
        1
    );
    assert_eq!(page.locator("xpath=//li").count().await.unwrap(), 3);
    assert_eq!(
        page.locator("ul")
            .locator("li")
            .nth(1)
            .inner_text()
            .await
            .unwrap(),
        "Two bold"
    );
    assert_eq!(
        page.locator("li").first().inner_text().await.unwrap(),
        "One"
    );
    assert_eq!(
        page.locator("li").last().inner_text().await.unwrap(),
        "Three"
    );
    let with_bold = Filter {
        has: Some("b".into()),
        ..Default::default()
    };
    assert_eq!(
        page.locator("li")
            .filter(with_bold)
            .inner_text()
            .await
            .unwrap(),
        "Two bold"
    );
    let not_two = Filter {
        has_not_text: Some("Two".into()),
        ..Default::default()
    };
    assert_eq!(page.locator("li").filter(not_two).count().await.unwrap(), 2);
    assert_eq!(
        page.locator("li").all_inner_texts().await.unwrap(),
        vec!["One", "Two bold", "Three"]
    );
    assert_eq!(page.locator("li").all().await.unwrap().len(), 3);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn strict_mode_violations_are_reported() {
    let (browser, page) = page_with(FORM).await;
    let err = page.locator("li").inner_text().await.unwrap_err();
    assert!(err.to_string().contains("strict mode violation"), "{err}");
    assert!(err.to_string().contains("resolved to 3 elements"), "{err}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn click_waits_for_the_element_to_appear_and_be_enabled() {
    let (browser, page) = page_with(
        "<script>setTimeout(() => { const b = document.createElement('button'); b.textContent = 'Late'; b.disabled = true;
          b.onclick = () => window.clicked = true; document.body.appendChild(b);
          setTimeout(() => b.disabled = false, 300); }, 200)</script>",
    )
    .await;
    page.get_by_text("Late", true).click().await.unwrap();
    assert!(
        page.evaluate::<bool>("window.clicked === true")
            .await
            .unwrap()
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn actions_fill_press_check_select_upload_hover() {
    let (browser, page) = page_with(FORM).await;
    let email = page.get_by_label("Email", false);
    email.fill("a@b.c").await.unwrap();
    assert_eq!(email.input_value().await.unwrap(), "a@b.c");
    email.press("Backspace").await.unwrap();
    email.press_sequentially("om").await.unwrap();
    assert_eq!(email.input_value().await.unwrap(), "a@b.com");
    email.clear().await.unwrap();
    assert_eq!(email.input_value().await.unwrap(), "");

    let terms = page.locator("#terms");
    assert!(!terms.is_checked().await.unwrap());
    terms.check().await.unwrap();
    assert!(terms.is_checked().await.unwrap());
    terms.check().await.unwrap();
    terms.uncheck().await.unwrap();
    assert!(!terms.is_checked().await.unwrap());
    page.get_by_text("Accept terms", false)
        .click()
        .await
        .unwrap();
    assert!(terms.is_checked().await.unwrap());

    assert_eq!(
        page.locator("#color")
            .select_option(&["Blue"])
            .await
            .unwrap(),
        vec!["b"]
    );
    assert_eq!(page.locator("#color").input_value().await.unwrap(), "b");
    let err = page
        .locator("#color")
        .select_option(&["Purple"])
        .timeout(Duration::from_millis(300))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Timeout { .. }), "{err:?}");

    let file = std::env::temp_dir().join(format!("cox-upload-{}.txt", std::process::id()));
    std::fs::write(&file, "hello").unwrap();
    page.locator("#upload")
        .set_input_files(&[file.clone()])
        .await
        .unwrap();
    assert_eq!(
        page.evaluate::<String>("document.getElementById('upload').files[0].name")
            .await
            .unwrap(),
        file.file_name().unwrap().to_string_lossy()
    );
    let _ = std::fs::remove_file(file);

    page.locator("#hover-target").hover().await.unwrap();
    assert!(
        page.evaluate::<bool>("window.hovered === true")
            .await
            .unwrap()
    );
    page.get_by_test_id("submit").dblclick().await.unwrap();
    assert_eq!(page.evaluate::<i64>("window.submitted").await.unwrap(), 2);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn states_queries_and_waiting() {
    let (browser, page) = page_with(FORM).await;
    let disabled = page.get_by_role(
        "button",
        RoleOptions {
            name: Some("Disabled".into()),
            ..Default::default()
        },
    );
    assert!(disabled.is_disabled().await.unwrap());
    assert!(!disabled.is_enabled().await.unwrap());
    assert!(page.locator("#email").is_editable().await.unwrap());
    assert!(page.locator("h1").is_visible().await.unwrap());
    assert!(page.locator("#missing").is_hidden().await.unwrap());
    assert!(!page.locator("#missing").is_visible().await.unwrap());
    assert_eq!(
        page.locator("ul").inner_html().await.unwrap(),
        "<li>One</li><li>Two <b>bold</b></li><li>Three</li>"
    );
    let n: i64 = page
        .locator("ul")
        .evaluate_with("(el, k) => el.children.length * k", 2)
        .await
        .unwrap();
    assert_eq!(n, 6);
    let bbox = page
        .locator("#hover-target")
        .bounding_box()
        .await
        .unwrap()
        .unwrap();
    assert_eq!((bbox.width, bbox.height), (50.0, 20.0));

    page.evaluate::<()>("() => setTimeout(() => document.querySelector('h1').remove(), 200)")
        .await
        .unwrap();
    page.locator("h1")
        .wait_for(WaitForState::Detached)
        .await
        .unwrap();
    page.evaluate::<()>("() => setTimeout(() => { const p = document.createElement('p'); p.id = 'late'; p.textContent = 'x'; document.body.append(p) }, 200)")
        .await
        .unwrap();
    page.locator("#late")
        .wait_for(WaitForState::Visible)
        .await
        .unwrap();
    let err = page
        .locator("#never")
        .wait_for(WaitForState::Attached)
        .timeout(Duration::from_millis(200))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Timeout { .. }), "{err:?}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn covered_elements_are_not_clicked() {
    let (browser, page) = page_with(
        "<button id=b onclick='window.clicked=true' style='position:absolute;top:10px;left:10px'>under</button>
         <div id=overlay style='position:fixed;inset:0;background:rgba(0,0,0,.1)'></div>",
    )
    .await;
    let err = page
        .locator("#b")
        .click()
        .timeout(Duration::from_millis(500))
        .await
        .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("intercepts pointer events"), "{text}");
    assert!(
        page.evaluate::<Option<bool>>("window.clicked")
            .await
            .unwrap()
            .is_none()
    );
    page.locator("#b").click().force(true).await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn element_screenshot_and_scrolling() {
    let (browser, page) = page_with(
        "<div style='height:2000px'></div><div id=t style='width:30px;height:15px;background:rgb(0,128,0)'></div>",
    )
    .await;
    page.locator("#t")
        .scroll_into_view_if_needed()
        .await
        .unwrap();
    assert!(page.evaluate::<f64>("window.scrollY").await.unwrap() > 1000.0);
    let png = page.locator("#t").screenshot().await.unwrap();
    let (w, h, _) = crate::screenshot::decode(&png);
    assert_eq!((w, h), (30, 15));
    page.locator("#t").dispatch_event("click").await.unwrap();
    browser.close().await.unwrap();
}
