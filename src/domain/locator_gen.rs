//! Playwright's `asLocator('javascript', selector)` (`locatorGenerators.ts`
//! with its selector parser): the locator code a selector reads as, e.g.
//! `getByRole('button', { name: 'Go' })`.

/// The JavaScript locator for `selector`, or the selector itself when it does
/// not parse (as Playwright does).
pub fn as_locator(selector: &str) -> String {
    parse_selector(selector)
        .and_then(|parts| inner_as_locator(&parts))
        .unwrap_or_else(|_| selector.to_owned())
}

type R<T> = Result<T, ()>;

#[derive(Debug, Clone, PartialEq)]
enum Body {
    Text(String),
    Nested(Vec<Part>),
}

#[derive(Debug, Clone, PartialEq)]
struct Part {
    name: String,
    body: Body,
    /// The text after `name=`.
    source: String,
}

impl Part {
    fn text(&self) -> &str {
        match &self.body {
            Body::Text(t) => t,
            Body::Nested(_) => "",
        }
    }
}

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

/// Playwright's `parseSelectorString`: `(name, body)` per `>>` part.
fn parse_selector_string(selector: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = selector.chars().collect();
    let mut parts = Vec::new();
    let append = |parts: &mut Vec<(String, String)>, start: usize, end: usize| {
        let part: String = chars[start..end]
            .iter()
            .collect::<String>()
            .trim()
            .to_owned();
        let is_name = |n: &str| {
            !n.is_empty()
                && n.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-+:*".contains(c))
        };
        let (name, body) = match part.find('=') {
            Some(eq) if is_name(part[..eq].trim()) => {
                (part[..eq].trim().to_owned(), part[eq + 1..].to_owned())
            }
            _ if part.chars().count() > 1
                && ((part.starts_with('"') && part.ends_with('"'))
                    || (part.starts_with('\'') && part.ends_with('\''))) =>
            {
                ("text".to_owned(), part.clone())
            }
            _ if part.trim_start_matches('(').starts_with("//") || part.starts_with("..") => {
                ("xpath".to_owned(), part.clone())
            }
            _ => ("css".to_owned(), part.clone()),
        };
        let name = name.strip_prefix('*').map(str::to_owned).unwrap_or(name);
        parts.push((name, body));
    };
    if !selector.contains(">>") {
        append(&mut parts, 0, chars.len());
        return parts;
    }
    let (mut index, mut start) = (0usize, 0usize);
    let mut quote: Option<char> = None;
    let ignore_text_quote = |start: usize, index: usize| {
        let prefix: String = chars[start..index].iter().collect();
        let t = prefix.trim_start();
        t.strip_prefix("text")
            .map(str::trim_start)
            .and_then(|r| r.strip_prefix('='))
            .is_some_and(|rest| !rest.is_empty())
    };
    while index < chars.len() {
        let c = chars[index];
        if c == '\\' && index + 1 < chars.len() {
            index += 2;
        } else if Some(c) == quote {
            quote = None;
            index += 1;
        } else if quote.is_none()
            && (c == '"' || c == '\'' || c == '`')
            && !ignore_text_quote(start, index)
        {
            quote = Some(c);
            index += 1;
        } else if quote.is_none() && c == '>' && chars.get(index + 1) == Some(&'>') {
            append(&mut parts, start, index);
            index += 2;
            start = index;
        } else {
            index += 1;
        }
    }
    append(&mut parts, start, chars.len());
    parts
}

fn stringify(parts: &[Part], force_engine: bool) -> String {
    parts
        .iter()
        .map(|p| {
            let include = force_engine
                || !(p.name == "css"
                    || (p.name == "xpath"
                        && (p.source.starts_with("//") || p.source.starts_with(".."))));
            if include {
                format!("{}={}", p.name, p.source)
            } else {
                p.source.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" >> ")
}

fn is_enter_frame(p: &Part) -> bool {
    p.name == "internal:control" && p.text() == "enter-frame"
}

/// Playwright's `parseSelector` (CSS parts are kept as text).
fn parse_selector(selector: &str) -> R<Vec<Part>> {
    let mut parts: Vec<Part> = Vec::new();
    for (name, body) in parse_selector_string(selector) {
        if name == "css" || name == "css:light" {
            let body = if name == "css:light" {
                format!(":light({body})")
            } else {
                body
            };
            parts.push(Part {
                name: "css".into(),
                body: Body::Text(body.clone()),
                source: body,
            });
            continue;
        }
        if NESTED.contains(&name.as_str()) {
            let unescaped: Vec<serde_json::Value> =
                serde_json::from_str(&format!("[{body}]")).map_err(|_| ())?;
            let inner = unescaped.first().and_then(|v| v.as_str()).ok_or(())?;
            if unescaped.len() > 2 {
                return Err(());
            }
            let mut nested = parse_selector(inner)?;
            // A nested selector repeating the outer frames is relative to them.
            if let Some(last_frame) = nested.iter().rposition(is_enter_frame) {
                let outer: Vec<Part> = match parts.first() {
                    Some(p) if p.name == "internal:control" && p.text() == "any-frame" => {
                        parts[1..].to_vec()
                    }
                    _ => parts.clone(),
                };
                if outer.len() > last_frame
                    && stringify(&nested[..=last_frame], false)
                        == stringify(&outer[..=last_frame], false)
                {
                    nested.drain(..=last_frame);
                }
            }
            parts.push(Part {
                name,
                body: Body::Nested(nested),
                source: body,
            });
            continue;
        }
        parts.push(Part {
            name,
            body: Body::Text(body.clone()),
            source: body,
        });
    }
    if parts
        .first()
        .is_some_and(|p| NESTED.contains(&p.name.as_str()))
    {
        return Err(());
    }
    Ok(parts)
}

/// A text argument: a string (exact or not) or a regular expression.
#[derive(Debug, Clone, PartialEq)]
enum TextArg {
    Str(String, bool),
    Regex(String, String),
}

/// `String(new RegExp(source, flags))`: `/` escaped outside classes, line
/// breaks escaped, flags in canonical order.
fn regex_string(source: &str, flags: &str) -> String {
    let mut out = String::new();
    let (mut escaped, mut in_class) = (false, false);
    for c in source.chars() {
        if escaped {
            out.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' => {
                escaped = true;
                out.push(c);
            }
            '[' => {
                in_class = true;
                out.push(c);
            }
            ']' => {
                in_class = false;
                out.push(c);
            }
            '/' if !in_class => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    if out.is_empty() {
        out = "(?:)".into();
    }
    let flags: String = "dgimsuvy".chars().filter(|f| flags.contains(*f)).collect();
    format!("/{out}/{flags}")
}

/// Playwright's `normalizeEscapedRegexQuotes`: `\'` `\"` `` \` `` lose the
/// backslash (unless it is itself escaped).
fn normalize_regex_quotes(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            let mut j = i;
            while j < chars.len() && chars[j] == '\\' {
                j += 1;
            }
            let run = j - i;
            if run % 2 == 1 && j < chars.len() && matches!(chars[j], '\'' | '"' | '`') {
                out.extend(std::iter::repeat_n('\\', run - 1));
                out.push(chars[j]);
                i = j + 1;
                continue;
            }
            out.extend(std::iter::repeat_n('\\', run));
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn regex_source(source: &str, flags: &str) -> String {
    normalize_regex_quotes(&regex_string(source, flags))
}

/// Playwright's `detectExact`.
fn detect_exact(text: &str) -> R<TextArg> {
    if let Some(rest) = text.strip_prefix('/')
        && !text.contains('\n')
        && let Some(slash) = rest.rfind('/')
    {
        let flags = &rest[slash + 1..];
        if flags.chars().all(|c| "igm".contains(c)) {
            return Ok(TextArg::Regex(rest[..slash].to_owned(), flags.to_owned()));
        }
    }
    let json = |t: &str| serde_json::from_str::<String>(t).map_err(|_| ());
    if text.ends_with('"') {
        return Ok(TextArg::Str(json(text)?, true));
    }
    if let Some(t) = text.strip_suffix('s').filter(|t| t.ends_with('"')) {
        return Ok(TextArg::Str(json(t)?, true));
    }
    if let Some(t) = text.strip_suffix('i').filter(|t| t.ends_with('"')) {
        return Ok(TextArg::Str(json(t)?, false));
    }
    Ok(TextArg::Str(text.to_owned(), false))
}

/// Playwright's `escapeWithQuotes(text, "'")`.
fn quote(text: &str) -> String {
    let json = serde_json::to_string(text).unwrap_or_default();
    let inner = &json[1..json.len() - 1];
    format!("'{}'", inner.replace("\\\"", "\"").replace('\'', "\\'"))
}

fn text_arg(t: &TextArg) -> String {
    match t {
        TextArg::Regex(s, f) => regex_source(s, f),
        TextArg::Str(s, _) => quote(s),
    }
}

fn call_with_exact(method: &str, t: &TextArg) -> String {
    match t {
        TextArg::Regex(..) => format!("{method}({})", text_arg(t)),
        TextArg::Str(s, true) => format!("{method}({}, {{ exact: true }})", quote(s)),
        TextArg::Str(s, false) => format!("{method}({})", quote(s)),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum AttrValue {
    Truthy,
    Str(String),
    Bool(bool),
    Regex(String, String),
}

#[derive(Debug, Clone, PartialEq)]
struct Attr {
    name: String,
    value: AttrValue,
    case_sensitive: bool,
}

/// Playwright's `parseAttributeSelector(selector, allowUnquotedStrings: true)`.
fn parse_attribute_selector(selector: &str) -> R<(String, Vec<Attr>)> {
    let c: Vec<char> = selector.chars().collect();
    let mut wp = 0usize;
    let next = |wp: usize| c.get(wp).copied();
    let is_name_char =
        |ch: char| ch >= '\u{80}' || ch.is_ascii_alphanumeric() || ch == '_' || ch == '-';
    let skip = |wp: &mut usize| {
        while next(*wp).is_some_and(char::is_whitespace) {
            *wp += 1;
        }
    };
    let ident = |wp: &mut usize| {
        skip(wp);
        let mut s = String::new();
        while let Some(ch) = next(*wp).filter(|ch| is_name_char(*ch)) {
            s.push(ch);
            *wp += 1;
        }
        s
    };
    let quoted = |wp: &mut usize| -> R<String> {
        let q = next(*wp).ok_or(())?;
        *wp += 1;
        let mut s = String::new();
        while let Some(ch) = next(*wp).filter(|ch| *ch != q) {
            if ch == '\\' {
                *wp += 1;
            }
            s.push(next(*wp).ok_or(())?);
            *wp += 1;
        }
        if next(*wp) != Some(q) {
            return Err(());
        }
        *wp += 1;
        Ok(s)
    };
    let name = ident(&mut wp);
    skip(&mut wp);
    let mut attrs = Vec::new();
    while next(wp) == Some('[') {
        wp += 1;
        let mut path = Vec::new();
        loop {
            skip(&mut wp);
            let token = if matches!(next(wp), Some('\'' | '"')) {
                quoted(&mut wp)?
            } else {
                ident(&mut wp)
            };
            if token.is_empty() {
                return Err(());
            }
            path.push(token);
            skip(&mut wp);
            if next(wp) == Some('.') {
                wp += 1;
                continue;
            }
            break;
        }
        let attr_name = path.join(".");
        if next(wp) == Some(']') {
            wp += 1;
            attrs.push(Attr {
                name: attr_name,
                value: AttrValue::Truthy,
                case_sensitive: false,
            });
            skip(&mut wp);
            continue;
        }
        skip(&mut wp);
        let mut op = String::new();
        if let Some(ch) = next(wp) {
            op.push(ch);
            wp += 1;
        }
        if op != "="
            && let Some(ch) = next(wp)
        {
            op.push(ch);
            wp += 1;
        }
        if !["=", "*=", "^=", "$=", "|=", "~="].contains(&op.as_str()) {
            return Err(());
        }
        skip(&mut wp);
        let mut case_sensitive = true;
        let value = match next(wp) {
            Some('/') => {
                wp += 1;
                let mut source = String::new();
                let mut in_class = false;
                loop {
                    let ch = next(wp).ok_or(())?;
                    if ch == '\\' {
                        source.push(ch);
                        wp += 1;
                        source.push(next(wp).ok_or(())?);
                        wp += 1;
                        continue;
                    }
                    if in_class && ch == ']' {
                        in_class = false;
                    } else if !in_class && ch == '[' {
                        in_class = true;
                    } else if !in_class && ch == '/' {
                        break;
                    }
                    source.push(ch);
                    wp += 1;
                }
                wp += 1;
                let mut flags = String::new();
                while let Some(f) = next(wp).filter(|f| "dgimsuvy".contains(*f)) {
                    flags.push(f);
                    wp += 1;
                }
                AttrValue::Regex(source, flags)
            }
            Some('\'' | '"') => {
                let v = quoted(&mut wp)?;
                skip(&mut wp);
                match next(wp) {
                    Some('i' | 'I') => {
                        case_sensitive = false;
                        wp += 1;
                    }
                    Some('s' | 'S') => wp += 1,
                    _ => {}
                }
                AttrValue::Str(v)
            }
            _ => {
                let mut v = String::new();
                while let Some(ch) =
                    next(wp).filter(|ch| is_name_char(*ch) || *ch == '+' || *ch == '.')
                {
                    v.push(ch);
                    wp += 1;
                }
                match v.as_str() {
                    "true" => AttrValue::Bool(true),
                    "false" => AttrValue::Bool(false),
                    _ => AttrValue::Str(v),
                }
            }
        };
        skip(&mut wp);
        if next(wp) != Some(']') {
            return Err(());
        }
        wp += 1;
        attrs.push(Attr {
            name: attr_name,
            value,
            case_sensitive,
        });
        skip(&mut wp);
    }
    if wp < c.len() || (name.is_empty() && attrs.is_empty()) {
        return Err(());
    }
    Ok((name, attrs))
}

fn role_locator(body: &str) -> R<String> {
    let (role, attrs) = parse_attribute_selector(body)?;
    let mut name: Option<&AttrValue> = None;
    let mut description: Option<&AttrValue> = None;
    let mut exact: Option<bool> = None;
    let mut rest = Vec::new();
    for a in &attrs {
        match a.name.as_str() {
            "name" | "description" => {
                if exact.is_some_and(|e| e != a.case_sensitive) {
                    return Err(());
                }
                exact = Some(a.case_sensitive);
                if a.name == "name" {
                    name = Some(&a.value);
                } else {
                    description = Some(&a.value);
                }
            }
            other => {
                let key = if other == "include-hidden" {
                    "includeHidden"
                } else {
                    other
                };
                let value = match &a.value {
                    AttrValue::Str(s) if other == "level" => s.parse::<f64>().map_or_else(
                        |_| "NaN".to_owned(),
                        |n| {
                            if n.fract() == 0.0 {
                                format!("{n:.0}")
                            } else {
                                n.to_string()
                            }
                        },
                    ),
                    AttrValue::Str(s) => quote(s),
                    AttrValue::Bool(b) => b.to_string(),
                    AttrValue::Regex(s, f) => regex_string(s, f),
                    AttrValue::Truthy => "null".into(),
                };
                rest.push(format!("{key}: {value}"));
            }
        }
    }
    let mut out = Vec::new();
    let string_valued = |v: Option<&AttrValue>| matches!(v, Some(AttrValue::Str(_)));
    for (key, v) in [("name", name), ("description", description)] {
        match v {
            Some(AttrValue::Regex(s, f)) => out.push(format!("{key}: {}", regex_source(s, f))),
            Some(AttrValue::Str(s)) => out.push(format!("{key}: {}", quote(s))),
            _ => {}
        }
    }
    if exact == Some(true) && (string_valued(name) || string_valued(description)) {
        out.push("exact: true".into());
    }
    out.extend(rest);
    let attrs = if out.is_empty() {
        String::new()
    } else {
        format!(", {{ {} }}", out.join(", "))
    };
    Ok(format!("getByRole({}{attrs})", quote(&role)))
}

/// Playwright's `innerAsLocators(...)[0]`.
fn inner_as_locator(parts: &[Part]) -> R<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut index = 0;
    while index < parts.len() {
        let part = &parts[index];
        let body = part.text();
        let nested = |p: &Part| -> R<String> {
            match &p.body {
                Body::Nested(inner) => inner_as_locator(inner),
                Body::Text(_) => Err(()),
            }
        };
        let token = match part.name.as_str() {
            "internal:describe" => {
                index += 1;
                continue;
            }
            "nth" => match body {
                "0" => "first()".to_owned(),
                "-1" => "last()".to_owned(),
                n => format!("nth({n})"),
            },
            "visible" => {
                if body == "true" {
                    "visible()".to_owned()
                } else {
                    "filter({ visible: false })".to_owned()
                }
            }
            "internal:text" => call_with_exact("getByText", &detect_exact(body)?),
            "internal:has-text" | "internal:has-not-text"
                if !matches!(detect_exact(body)?, TextArg::Str(_, true)) =>
            {
                let key = if part.name == "internal:has-text" {
                    "hasText"
                } else {
                    "hasNotText"
                };
                format!("filter({{ {key}: {} }})", text_arg(&detect_exact(body)?))
            }
            "internal:has" => format!("filter({{ has: {} }})", nested(part)?),
            "internal:has-not" => format!("filter({{ hasNot: {} }})", nested(part)?),
            "internal:and" => format!("and({})", nested(part)?),
            "internal:or" => format!("or({})", nested(part)?),
            "internal:chain" => format!("locator({})", nested(part)?),
            "internal:label" => call_with_exact("getByLabel", &detect_exact(body)?),
            "internal:role" => role_locator(body)?,
            "internal:testid" => {
                let (_, attrs) = parse_attribute_selector(body)?;
                match &attrs.first().ok_or(())?.value {
                    AttrValue::Regex(s, f) => format!("getByTestId({})", regex_source(s, f)),
                    AttrValue::Str(s) => format!("getByTestId({})", quote(s)),
                    _ => return Err(()),
                }
            }
            "internal:attr"
                if parse_attribute_selector(body)
                    .ok()
                    .and_then(|(_, a)| a.first().map(|a| a.name.clone()))
                    .is_some_and(|n| matches!(n.as_str(), "placeholder" | "alt" | "title")) =>
            {
                let (_, attrs) = parse_attribute_selector(body)?;
                let a = &attrs[0];
                let method = match a.name.as_str() {
                    "placeholder" => "getByPlaceholder",
                    "alt" => "getByAltText",
                    _ => "getByTitle",
                };
                let arg = match &a.value {
                    AttrValue::Regex(s, f) => TextArg::Regex(s.clone(), f.clone()),
                    AttrValue::Str(s) => TextArg::Str(s.clone(), a.case_sensitive),
                    _ => return Err(()),
                };
                call_with_exact(method, &arg)
            }
            "internal:control" if body == "any-frame" => "frameLocator()".to_owned(),
            "internal:control" if body == "enter-frame" => {
                let last = tokens.last_mut().ok_or(())?;
                last.push_str(".contentFrame()");
                index += 1;
                continue;
            }
            _ => {
                let locator = format!(
                    "locator({})",
                    quote(&stringify(std::slice::from_ref(part), false))
                );
                if let Some(next) = parts.get(index + 1)
                    && matches!(
                        next.name.as_str(),
                        "internal:has-text" | "internal:has-not-text"
                    )
                    && !matches!(detect_exact(next.text())?, TextArg::Str(_, true))
                {
                    let key = if next.name == "internal:has-text" {
                        "hasText"
                    } else {
                        "hasNotText"
                    };
                    tokens.push(format!(
                        "{locator}.filter({{ {key}: {} }})",
                        text_arg(&detect_exact(next.text())?)
                    ));
                    index += 2;
                    continue;
                }
                locator
            }
        };
        tokens.push(token);
        index += 1;
    }
    Ok(tokens.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Selectors built by Playwright 1.63's own locators, with its asLocator.
    const CASES: &str = include_str!("data/as_locator_cases.json");

    #[test]
    fn matches_playwright() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(CASES).unwrap();
        for c in cases {
            let selector = c["selector"].as_str().unwrap();
            assert_eq!(
                as_locator(selector),
                c["locator"].as_str().unwrap(),
                "{selector}"
            );
        }
    }
}
