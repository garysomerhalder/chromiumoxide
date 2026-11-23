//! Playwright 1.66's `locateOnlyVisible` context option: the selector a
//! locator queries with gets `visible=true` at the end and before each
//! `nth=` (`first()`, `last()`, `nth()`), unless that position already has
//! an explicit visibility filter. Mirrors `restrictToVisible` in Playwright's
//! client `locator.ts`, on its selector parser (`selectorParser.ts`).

/// Engines whose body is a nested selector (Playwright's `kNestedSelectorNames`).
const NESTED: &[&str] = &[
    "internal:has",
    "internal:has-not",
    "internal:and",
    "internal:or",
    "internal:chain",
    "left-of",
    "right-of",
    "above",
    "below",
    "near",
];
const WITH_DISTANCE: &[&str] = &["left-of", "right-of", "above", "below", "near"];

#[derive(Debug, Clone)]
struct Part {
    name: String,
    source: String,
    nested: Option<Nested>,
}

#[derive(Debug, Clone)]
struct Nested {
    parsed: Parsed,
    distance: Option<serde_json::Number>,
}

#[derive(Debug, Clone)]
struct Parsed {
    parts: Vec<Part>,
    capture: Option<usize>,
}

/// `locator.selectorForQuery()` with `locateOnlyVisible`. A selector that
/// does not parse is returned as it is (the query reports the error).
pub fn restrict_to_visible(selector: &str) -> String {
    match parse(selector) {
        Some(parsed) => stringify(&restrict(&parsed, true)),
        None => selector.to_owned(),
    }
}

fn restrict(selector: &Parsed, restrict_result: bool) -> Parsed {
    fn add_visible_unless_explicit(parts: &mut Vec<Part>) {
        let last = parts
            .iter()
            .rev()
            .find(|p| p.name != "internal:describe" && p.name != "nth");
        if last.is_some_and(|p| p.name != "visible") {
            parts.push(Part {
                name: "visible".into(),
                source: "true".into(),
                nested: None,
            });
        }
    }
    let mut parts = Vec::new();
    let mut capture = None;
    for (index, part) in selector.parts.iter().enumerate() {
        if part.name == "nth" {
            add_visible_unless_explicit(&mut parts);
        }
        if Some(index) == selector.capture {
            capture = Some(parts.len());
        }
        parts.push(restrict_nested(part));
    }
    if restrict_result {
        add_visible_unless_explicit(&mut parts);
    }
    Parsed { parts, capture }
}

fn restrict_nested(part: &Part) -> Part {
    let Some(nested) = &part.nested else {
        return part.clone();
    };
    let parsed = restrict(&nested.parsed, false);
    if parsed.parts.len() == nested.parsed.parts.len() {
        return part.clone();
    }
    let inner = serde_json::to_string(&stringify(&parsed)).unwrap_or_default();
    let source = match &nested.distance {
        None => inner,
        Some(d) => format!("{inner}, {d}"),
    };
    Part {
        name: part.name.clone(),
        source,
        nested: Some(Nested {
            parsed,
            distance: nested.distance.clone(),
        }),
    }
}

/// Playwright's `stringifySelector`.
fn stringify(selector: &Parsed) -> String {
    selector
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let captured = Some(i) == selector.capture;
            let include_engine = captured
                || !(p.name == "css"
                    || p.name == "xpath"
                        && (p.source.starts_with("//") || p.source.starts_with("..")));
            format!(
                "{}{}{}",
                if captured { "*" } else { "" },
                if include_engine {
                    format!("{}=", p.name)
                } else {
                    String::new()
                },
                p.source
            )
        })
        .collect::<Vec<_>>()
        .join(" >> ")
}

/// Playwright's `parseSelector` (without parsing CSS itself).
fn parse(selector: &str) -> Option<Parsed> {
    let (raw, capture) = split(selector)?;
    let mut parts: Vec<Part> = Vec::new();
    for (name, body) in raw {
        if name == "css" || name == "css:light" {
            let source = if name == "css:light" {
                format!(":light({body})")
            } else {
                body
            };
            parts.push(Part {
                name: "css".into(),
                source,
                nested: None,
            });
            continue;
        }
        if NESTED.contains(&name.as_str()) {
            let values: Vec<serde_json::Value> = serde_json::from_str(&format!("[{body}]")).ok()?;
            let inner = match values.as_slice() {
                [serde_json::Value::String(s)] | [serde_json::Value::String(s), _] => s,
                _ => return None,
            };
            let distance = match values.get(1) {
                None => None,
                Some(serde_json::Value::Number(n)) if WITH_DISTANCE.contains(&name.as_str()) => {
                    Some(n.clone())
                }
                Some(_) => return None,
            };
            let mut parsed = parse(inner)?;
            // An inner selector repeating the outer frame hops drops them.
            let is_enter = |p: &Part| p.name == "internal:control" && p.source == "enter-frame";
            if let Some(last_frame) = parsed.parts.iter().rposition(is_enter) {
                let outer: &[Part] = match parts.first() {
                    Some(p) if p.name == "internal:control" && p.source == "any-frame" => {
                        &parts[1..]
                    }
                    _ => &parts[..],
                };
                let prefix = |list: &[Part]| {
                    stringify(&Parsed {
                        parts: list.iter().take(last_frame + 1).cloned().collect(),
                        capture: None,
                    })
                };
                if prefix(&parsed.parts) == prefix(outer) {
                    parsed.parts.drain(..=last_frame);
                    if let Some(c) = parsed.capture {
                        if c <= last_frame {
                            return None;
                        }
                        parsed.capture = Some(c - last_frame - 1);
                    }
                }
            }
            parts.push(Part {
                name,
                source: body,
                nested: Some(Nested { parsed, distance }),
            });
            continue;
        }
        parts.push(Part {
            name,
            source: body,
            nested: None,
        });
    }
    if parts
        .first()
        .is_some_and(|p| NESTED.contains(&p.name.as_str()))
    {
        return None;
    }
    Some(Parsed { parts, capture })
}

/// Playwright's `parseSelectorString`: `(name, body)` parts and the index
/// of the `*` capture.
#[allow(clippy::type_complexity)]
fn split(selector: &str) -> Option<(Vec<(String, String)>, Option<usize>)> {
    let chars: Vec<char> = selector.chars().collect();
    let mut parts = Vec::new();
    let mut capture = None;
    let mut append = |start: usize, end: usize| -> Option<()> {
        let text: String = chars[start..end].iter().collect();
        let part = text.trim();
        let named = part.find('=').and_then(|eq| {
            let name = part[..eq].trim();
            let ok = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-+:*".contains(c));
            ok.then(|| (name.to_owned(), part[eq + 1..].to_owned()))
        });
        let quoted = |q: char| part.chars().count() > 1 && part.starts_with(q) && part.ends_with(q);
        let (mut name, body) = match named {
            Some(nb) => nb,
            None if quoted('"') || quoted('\'') => ("text".to_owned(), part.to_owned()),
            None if part.trim_start_matches('(').starts_with("//") || part.starts_with("..") => {
                ("xpath".to_owned(), part.to_owned())
            }
            None => ("css".to_owned(), part.to_owned()),
        };
        if let Some(rest) = name.strip_prefix('*') {
            name = rest.to_owned();
            if capture.is_some() {
                return None;
            }
            capture = Some(parts.len());
        }
        parts.push((name, body));
        Some(())
    };
    if !selector.contains(">>") {
        append(0, chars.len())?;
        return Some((parts, capture));
    }
    let ignore_text_quote = |start: usize, index: usize| {
        let prefix: String = chars[start..index].iter().collect();
        let rest = prefix.trim_start();
        rest.strip_prefix("text")
            .map(str::trim_start)
            .and_then(|r| r.strip_prefix('='))
            .is_some_and(|r| !r.is_empty() && !r.contains('\n'))
    };
    let (mut index, mut start) = (0, 0);
    let mut quote: Option<char> = None;
    while index < chars.len() {
        let c = chars[index];
        if c == '\\' && index + 1 < chars.len() {
            index += 2;
        } else if Some(c) == quote {
            quote = None;
            index += 1;
        } else if quote.is_none()
            && matches!(c, '"' | '\'' | '`')
            && !ignore_text_quote(start, index)
        {
            quote = Some(c);
            index += 1;
        } else if quote.is_none() && c == '>' && chars.get(index + 1) == Some(&'>') {
            append(start, index)?;
            index += 2;
            start = index;
        } else {
            index += 1;
        }
    }
    append(start, index.min(chars.len()))?;
    Some((parts, capture))
}

#[cfg(test)]
mod tests {
    use super::restrict_to_visible;

    /// Recorded with the Playwright 1.66.0 alpha of 2026-11-22
    /// (`locator.selectorForQuery()` with `locateOnlyVisible: true`).
    const PLAYWRIGHT_1_66: &[(&str, &str)] = &[
        ("div", "div >> visible=true"),
        ("div >> nth=0", "div >> visible=true >> nth=0"),
        ("div >> nth=-1", "div >> visible=true >> nth=-1"),
        ("div >> nth=1", "div >> visible=true >> nth=1"),
        (
            "div >> visible=false >> nth=0",
            "div >> visible=false >> nth=0",
        ),
        ("div >> visible=true", "div >> visible=true"),
        (
            "section >> internal:role=button",
            "section >> internal:role=button >> visible=true",
        ),
        (
            "section >> internal:has=\"internal:role=button[name=\\\"Go\\\"i]\"",
            "section >> internal:has=\"internal:role=button[name=\\\"Go\\\"i]\" >> visible=true",
        ),
        (
            "section >> internal:has-not=\"b\"",
            "section >> internal:has-not=\"b\" >> visible=true",
        ),
        (
            "div >> internal:and=\"internal:text=\\\"Two\\\"i\"",
            "div >> internal:and=\"internal:text=\\\"Two\\\"i\" >> visible=true",
        ),
        (
            "div >> internal:or=\"span\"",
            "div >> internal:or=\"span\" >> visible=true",
        ),
        (
            "internal:text=\"Two\"i",
            "internal:text=\"Two\"i >> visible=true",
        ),
        (
            "div >> internal:describe=\"the divs\"",
            "div >> internal:describe=\"the divs\" >> visible=true",
        ),
        (
            "div >> nth=0 >> internal:describe=\"first div\"",
            "div >> visible=true >> nth=0 >> internal:describe=\"first div\"",
        ),
        (
            "iframe >> internal:control=enter-frame >> p",
            "iframe >> internal:control=enter-frame >> p >> visible=true",
        ),
        (
            "iframe >> internal:control=enter-frame >> p >> nth=0",
            "iframe >> internal:control=enter-frame >> p >> visible=true >> nth=0",
        ),
        (
            "section >> internal:has=\"button >> nth=0\"",
            "section >> internal:has=\"button >> visible=true >> nth=0\" >> visible=true",
        ),
        (
            "*css=section >> button",
            "*css=section >> button >> visible=true",
        ),
        ("button:near(div)", "button:near(div) >> visible=true"),
        ("//div", "//div >> visible=true"),
        ("text=\"a >> b\"", "text=\"a >> b\" >> visible=true"),
        (
            "button >> left-of=\"div\", 10",
            "button >> left-of=\"div\", 10 >> visible=true",
        ),
    ];

    #[test]
    fn matches_playwright_1_66() {
        for (selector, expected) in PLAYWRIGHT_1_66 {
            assert_eq!(restrict_to_visible(selector), *expected, "{selector}");
        }
    }
}
