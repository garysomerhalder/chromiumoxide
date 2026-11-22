//! PDF options as Playwright turns them into `Page.printToPDF` parameters.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_convert_to_inches_like_playwright() {
        assert_eq!(to_inches("96px").unwrap(), 1.0);
        assert_eq!(to_inches("192").unwrap(), 2.0);
        assert_eq!(to_inches("1in").unwrap(), 1.0);
        assert!((to_inches("2.54cm").unwrap() - 2.54 * 37.8 / 96.0).abs() < 1e-12);
        assert!((to_inches("10mm").unwrap() - 37.8 / 96.0).abs() < 1e-12);
        assert_eq!(
            to_inches("abc").unwrap_err(),
            "Failed to parse parameter value: abc"
        );
    }

    #[test]
    fn paper_formats() {
        assert_eq!(paper_format("Letter"), Some((8.5, 11.0)));
        assert_eq!(paper_format("a4"), Some((8.27, 11.7)));
        assert_eq!(paper_format("A0"), Some((33.1, 46.8)));
        assert_eq!(paper_format("b5"), None);
    }

    #[test]
    fn default_parameters_match_playwright() {
        let p = PdfOptions::default().to_protocol().unwrap();
        assert_eq!(
            p,
            serde_json::json!({
                "transferMode": "ReturnAsStream", "landscape": false, "displayHeaderFooter": false,
                "headerTemplate": "", "footerTemplate": "", "printBackground": false, "scale": 1.0,
                "paperWidth": 8.5, "paperHeight": 11.0, "marginTop": 0.0, "marginBottom": 0.0,
                "marginLeft": 0.0, "marginRight": 0.0, "pageRanges": "", "preferCSSPageSize": false,
                "generateTaggedPDF": false, "generateDocumentOutline": false
            })
        );
    }

    #[test]
    fn options_map_to_parameters() {
        let p = PdfOptions {
            format: Some("A4".into()),
            landscape: true,
            print_background: true,
            margin: Margin {
                top: Some("1in".into()),
                left: Some("48px".into()),
                ..Default::default()
            },
            page_ranges: "1-2".into(),
            ..Default::default()
        }
        .to_protocol()
        .unwrap();
        assert_eq!(
            (p["paperWidth"].as_f64(), p["paperHeight"].as_f64()),
            (Some(8.27), Some(11.7))
        );
        assert_eq!(p["marginTop"], 1.0);
        assert_eq!(p["marginLeft"], 0.5);
        assert_eq!(p["landscape"], true);
        assert_eq!(p["printBackground"], true);
        assert_eq!(p["pageRanges"], "1-2");
        let custom = PdfOptions {
            width: Some("4in".into()),
            height: Some("288px".into()),
            ..Default::default()
        }
        .to_protocol()
        .unwrap();
        assert_eq!(
            (
                custom["paperWidth"].as_f64(),
                custom["paperHeight"].as_f64()
            ),
            (Some(4.0), Some(3.0))
        );
        let bad = PdfOptions {
            format: Some("B5".into()),
            ..Default::default()
        };
        assert_eq!(bad.to_protocol().unwrap_err(), "Unknown paper format: B5");
    }
}
