//! PDF options as Playwright turns them into `Page.printToPDF` parameters.

use serde_json::{Value, json};

/// Page margins as CSS lengths (`"1in"`, `"10mm"`, `"20px"`, `"20"`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Margin {
    pub top: Option<String>,
    pub right: Option<String>,
    pub bottom: Option<String>,
    pub left: Option<String>,
}

/// Playwright's `page.pdf(options)` (the `path` option is up to the caller).
#[derive(Debug, Clone, PartialEq)]
pub struct PdfOptions {
    pub scale: f64,
    pub display_header_footer: bool,
    pub header_template: String,
    pub footer_template: String,
    pub print_background: bool,
    pub landscape: bool,
    /// e.g. `"1-5, 8"`; empty for all pages.
    pub page_ranges: String,
    /// `Letter`, `Legal`, `Tabloid`, `Ledger`, `A0`..`A6`; wins over width/height.
    pub format: Option<String>,
    pub width: Option<String>,
    pub height: Option<String>,
    pub margin: Margin,
    pub prefer_css_page_size: bool,
    pub tagged: bool,
    pub outline: bool,
}

impl Default for PdfOptions {
    fn default() -> Self {
        PdfOptions {
            scale: 1.0,
            display_header_footer: false,
            header_template: String::new(),
            footer_template: String::new(),
            print_background: false,
            landscape: false,
            page_ranges: String::new(),
            format: None,
            width: None,
            height: None,
            margin: Margin::default(),
            prefer_css_page_size: false,
            tagged: false,
            outline: false,
        }
    }
}

/// Paper size in inches for a Playwright format name (case-insensitive).
pub fn paper_format(name: &str) -> Option<(f64, f64)> {
    Some(match name.to_ascii_lowercase().as_str() {
        "letter" => (8.5, 11.0),
        "legal" => (8.5, 14.0),
        "tabloid" => (11.0, 17.0),
        "ledger" => (17.0, 11.0),
        "a0" => (33.1, 46.8),
        "a1" => (23.4, 33.1),
        "a2" => (16.54, 23.4),
        "a3" => (11.7, 16.54),
        "a4" => (8.27, 11.7),
        "a5" => (5.83, 8.27),
        "a6" => (4.13, 5.83),
        _ => return None,
    })
}

/// Playwright's `convertPrintParameterToInches`: px (default), in, cm, mm.
pub fn to_inches(text: &str) -> Result<f64, String> {
    let lower = text.to_ascii_lowercase();
    let (value, per_px) = match lower.len().checked_sub(2).map(|i| &lower[i..]) {
        Some("px") => (&text[..text.len() - 2], 1.0),
        Some("in") => (&text[..text.len() - 2], 96.0),
        Some("cm") => (&text[..text.len() - 2], 37.8),
        Some("mm") => (&text[..text.len() - 2], 3.78),
        _ => (text, 1.0),
    };
    let v: f64 = value
        .trim()
        .parse()
        .map_err(|_| format!("Failed to parse parameter value: {text}"))?;
    Ok(v * per_px / 96.0)
}

fn opt_inches(v: &Option<String>) -> Result<Option<f64>, String> {
    v.as_deref().map(to_inches).transpose()
}

impl PdfOptions {
    /// `Page.printToPDF` parameters, exactly as Playwright sends them.
    pub fn to_protocol(&self) -> Result<Value, String> {
        let (width, height) = match &self.format {
            Some(f) => paper_format(f).ok_or_else(|| format!("Unknown paper format: {f}"))?,
            None => (
                opt_inches(&self.width)?
                    .filter(|w| *w != 0.0)
                    .unwrap_or(8.5),
                opt_inches(&self.height)?
                    .filter(|h| *h != 0.0)
                    .unwrap_or(11.0),
            ),
        };
        let m = &self.margin;
        Ok(json!({
            "transferMode": "ReturnAsStream",
            "landscape": self.landscape,
            "displayHeaderFooter": self.display_header_footer,
            "headerTemplate": self.header_template,
            "footerTemplate": self.footer_template,
            "printBackground": self.print_background,
            "scale": self.scale,
            "paperWidth": width,
            "paperHeight": height,
            "marginTop": opt_inches(&m.top)?.unwrap_or(0.0),
            "marginBottom": opt_inches(&m.bottom)?.unwrap_or(0.0),
            "marginLeft": opt_inches(&m.left)?.unwrap_or(0.0),
            "marginRight": opt_inches(&m.right)?.unwrap_or(0.0),
            "pageRanges": self.page_ranges,
            "preferCSSPageSize": self.prefer_css_page_size,
            "generateTaggedPDF": self.tagged,
            "generateDocumentOutline": self.outline,
        }))
    }
}

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
