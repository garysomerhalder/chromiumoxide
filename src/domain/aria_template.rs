//! Aria snapshot templates (`toMatchAriaSnapshot`): a port of Playwright's
//! `parseAriaSnapshot`, with the YAML subset aria snapshots use, producing
//! the template object the injected script's `matchesExpectAriaTemplate`
//! takes.

use serde_json::{Value, json};

/// Parses a template; the error is Playwright's first error message.
pub fn parse_aria_template(text: &str) -> Result<Value, String> {
    let doc = Yaml::parse(text)?;
    let Some(Node::Seq(items)) = doc else {
        return Err("Aria snapshot must be a YAML sequence, elements starting with \" -\"".into());
    };
    let mut fragment = json!({"kind": "role", "role": "fragment"});
    convert_seq(&mut fragment, &items)?;
    let children = fragment.get("children").and_then(Value::as_array);
    let mode = fragment.get("containerMode").and_then(Value::as_str);
    if children.is_some_and(|c| c.len() == 1) && mode.is_none_or(|m| m == "contain") {
        return Ok(fragment["children"][0].clone());
    }
    Ok(fragment)
}

/// Playwright's `unshift`: drops blank lines and the common indentation.
pub fn unshift(snapshot: &str) -> String {
    let lines: Vec<&str> = snapshot
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .collect();
    let prefix = lines
        .iter()
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0)
        .min(100);
    lines
        .iter()
        .map(|l| l.get(prefix..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---- conversion (Playwright's convertSeq / convertMap) ----------------------

fn children(container: &mut Value) -> &mut Vec<Value> {
    if !container.get("children").is_some_and(Value::is_array) {
        container["children"] = json!([]);
    }
    container["children"].as_array_mut().expect("just set")
}

fn convert_seq(container: &mut Value, items: &[Node]) -> Result<(), String> {
    for item in items {
        match item {
            Node::Scalar(Scalar::Str(s)) => {
                let child = parse_key(s)?;
                children(container).push(child);
            }
            Node::Map(entries) => convert_map(container, entries)?,
            _ => return Err("Sequence items should be strings or maps".into()),
        }
    }
    Ok(())
}

fn convert_map(container: &mut Value, entries: &[(Node, Node)]) -> Result<(), String> {
    for (key, value) in entries {
        children(container);
        let Node::Scalar(Scalar::Str(key)) = key else {
            return Err("Only string keys are supported".into());
        };
        let string = match value {
            Node::Scalar(Scalar::Str(v)) => Some(v.as_str()),
            _ => None,
        };
        if key == "text" {
            let Some(v) = string else {
                return Err("Text value should be a string".into());
            };
            children(container).push(json!({"kind": "text", "text": text_value(v)}));
            continue;
        }
        if key == "/children" {
            match string {
                Some(v @ ("contain" | "equal" | "deep-equal")) => {
                    container["containerMode"] = json!(v);
                }
                _ => {
                    return Err(
                        "Strict value should be \"contain\", \"equal\" or \"deep-equal\"".into(),
                    );
                }
            }
            continue;
        }
        if let Some(prop) = key.strip_prefix('/') {
            let Some(v) = string else {
                return Err("Property value should be a string".into());
            };
            if !container.get("props").is_some_and(Value::is_object) {
                container["props"] = json!({});
            }
            container["props"][prop] = text_value(v);
            continue;
        }
        let mut child = parse_key(key)?;
        match value {
            Node::Scalar(s) => {
                let text = match s {
                    Scalar::Str(v) => v.clone(),
                    Scalar::Num(v) => v.clone(),
                    Scalar::Bool(b) => b.to_string(),
                    Scalar::Null => {
                        return Err("Node value should be a string or a sequence".into());
                    }
                };
                child["children"] = json!([{"kind": "text", "text": text_value(&text)}]);
                children(container).push(child);
            }
            Node::Seq(items) => {
                convert_seq(&mut child, items)?;
                children(container).push(child);
            }
            Node::Map(_) => return Err("Map values should be strings or sequences".into()),
        }
    }
    Ok(())
}

fn normalize_whitespace(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| *c != '\u{200b}' && *c != '\u{ad}')
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_value(value: &str) -> Value {
    json!({"raw": value, "normalized": normalize_whitespace(value)})
}

// ---- KeyParser ---------------------------------------------------------------

struct KeyParser {
    input: Vec<char>,
    pos: usize,
}

struct ParserError {
    message: String,
    pos: usize,
}

type KeyResult<T> = std::result::Result<T, ParserError>;

fn parse_key(text: &str) -> Result<Value, String> {
    let mut p = KeyParser {
        input: text.chars().collect(),
        pos: 0,
    };
    p.parse()
        .map_err(|e| format!("{}:\n\n{text}\n{}^\n", e.message, " ".repeat(e.pos)))
}

impl KeyParser {
    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }
    fn next(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }
    fn eof(&self) -> bool {
        self.pos >= self.input.len()
    }
    fn is_whitespace(&self) -> bool {
        self.peek().is_some_and(char::is_whitespace)
    }
    fn skip_whitespace(&mut self) {
        while self.is_whitespace() {
            self.pos += 1;
        }
    }
    fn error<T>(&self, message: &str, offset: usize) -> KeyResult<T> {
        Err(ParserError {
            message: message.to_owned(),
            pos: if offset == 0 { self.pos } else { offset },
        })
    }
    fn read_identifier(&mut self, kind: &str) -> KeyResult<String> {
        if self.eof() {
            return self.error(&format!("Unexpected end of input when expecting {kind}"), 0);
        }
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        Ok(self.input[start..self.pos].iter().collect())
    }
    fn read_string(&mut self) -> KeyResult<String> {
        let mut result = String::new();
        let mut escaped = false;
        while let Some(ch) = self.next() {
            if escaped {
                result.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                return Ok(result);
            } else {
                result.push(ch);
            }
        }
        self.error("Unterminated string", 0)
    }
    fn read_regex(&mut self) -> KeyResult<Value> {
        let mut result = String::new();
        let (mut escaped, mut inside_class) = (false, false);
        while let Some(ch) = self.next() {
            if escaped {
                result.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
                result.push(ch);
            } else if ch == '/' && !inside_class {
                return Ok(json!({"pattern": result}));
            } else if ch == '[' {
                inside_class = true;
                result.push(ch);
            } else if ch == ']' && inside_class {
                result.push(ch);
                inside_class = false;
            } else {
                result.push(ch);
            }
        }
        self.error("Unterminated regex", 0)
    }
    fn read_string_or_regex(&mut self) -> KeyResult<Option<Value>> {
        match self.peek() {
            Some('"') => {
                self.next();
                Ok(Some(json!(normalize_whitespace(&self.read_string()?))))
            }
            Some('/') => {
                self.next();
                Ok(Some(self.read_regex()?))
            }
            _ => Ok(None),
        }
    }
    fn read_attributes(&mut self, result: &mut Value) -> KeyResult<()> {
        loop {
            self.skip_whitespace();
            if self.peek() != Some('[') {
                return Ok(());
            }
            self.next();
            self.skip_whitespace();
            let mut error_pos = self.pos;
            let name = self.read_identifier("attribute")?;
            self.skip_whitespace();
            let mut value = String::new();
            if self.peek() == Some('=') {
                self.next();
                self.skip_whitespace();
                error_pos = self.pos;
                while self.peek().is_some_and(|c| c != ']' && !c.is_whitespace()) {
                    value.push(self.next().expect("peeked"));
                }
            }
            self.skip_whitespace();
            if self.peek() != Some(']') {
                return self.error("Expected ]", 0);
            }
            self.next();
            let value = if value.is_empty() {
                "true".to_owned()
            } else {
                value
            };
            self.apply_attribute(result, &name, &value, error_pos)?;
        }
    }
    fn parse(&mut self) -> KeyResult<Value> {
        self.skip_whitespace();
        let role = self.read_identifier("role")?;
        self.skip_whitespace();
        let name = self.read_string_or_regex()?.unwrap_or_else(|| json!(""));
        let mut result = json!({"kind": "role", "role": role, "name": name});
        self.read_attributes(&mut result)?;
        self.skip_whitespace();
        if !self.eof() {
            return self.error("Unexpected input", 0);
        }
        Ok(result)
    }
    fn check(&self, ok: bool, message: &str, pos: usize) -> KeyResult<()> {
        if ok { Ok(()) } else { self.error(message, pos) }
    }
    fn apply_attribute(
        &self,
        node: &mut Value,
        key: &str,
        value: &str,
        pos: usize,
    ) -> KeyResult<()> {
        let boolean = |v: &str| v == "true";
        let tri = |v: &str| match v {
            "true" => json!(true),
            "false" => json!(false),
            other => json!(other),
        };
        let is_bool = value == "true" || value == "false";
        match key {
            "checked" | "pressed" => {
                let msg = format!("Value of \"{key}\" attribute must be a boolean or \"mixed\"");
                self.check(is_bool || value == "mixed", &msg, pos)?;
                node[key] = tri(value);
            }
            "disabled" | "expanded" | "active" | "selected" => {
                let msg = format!("Value of \"{key}\" attribute must be a boolean");
                self.check(is_bool, &msg, pos)?;
                node[key] = json!(boolean(value));
            }
            "invalid" => {
                self.check(
                    is_bool || value == "grammar" || value == "spelling",
                    "Value of \"invalid\" attribute must be a boolean, \"grammar\" or \"spelling\"",
                    pos,
                )?;
                node[key] = tri(value);
            }
            "level" => {
                let n = js_number(value);
                self.check(
                    !n.is_nan(),
                    "Value of \"level\" attribute must be a number",
                    pos,
                )?;
                node[key] = serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number);
                if n.fract() == 0.0 && n.abs() < 9e15 {
                    node[key] = json!(n as i64);
                }
            }
            _ => self.check(false, &format!("Unsupported attribute [{key}]"), pos)?,
        }
        Ok(())
    }
}

/// JavaScript's `Number(string)` for the forms attribute values take.
fn js_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return i64::from_str_radix(h, 16).map_or(f64::NAN, |v| v as f64);
    }
    match t {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ if t.chars().all(|c| c.is_ascii_digit() || "+-.eE".contains(c)) => {
            t.parse().unwrap_or(f64::NAN)
        }
        _ => f64::NAN,
    }
}

/// JavaScript's `String(number)` for ordinary values.
fn js_number_string(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        }
    } else if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

// ---- the YAML subset ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Scalar {
    Str(String),
    /// A number, as JavaScript's `String()` prints it.
    Num(String),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Scalar(Scalar),
    Seq(Vec<Node>),
    Map(Vec<(Node, Node)>),
}

struct Line<'a> {
    indent: usize,
    text: &'a str,
}

struct Yaml<'a> {
    lines: Vec<Line<'a>>,
    pos: usize,
}

fn is_seq_entry(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

/// Cuts a ` #` comment off a plain scalar.
fn strip_comment(text: &str) -> &str {
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return &text[..i];
        }
    }
    text
}

/// Where a plain mapping key ends: the `:` followed by a space or the end.
fn plain_key_end(text: &str) -> Option<usize> {
    let content = strip_comment(text);
    let bytes = content.as_bytes();
    (0..bytes.len()).find(|&i| bytes[i] == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' '))
}

fn resolve_plain(s: &str) -> Scalar {
    let re = |p: &str| regex::Regex::new(p).expect("valid regex").is_match(s);
    if re(r"^(?:~|[Nn]ull|NULL)?$") {
        return Scalar::Null;
    }
    if re(r"^(?:[Tt]rue|TRUE)$") {
        return Scalar::Bool(true);
    }
    if re(r"^(?:[Ff]alse|FALSE)$") {
        return Scalar::Bool(false);
    }
    if re(r"^0o[0-7]+$") {
        return Scalar::Num(js_number_string(
            i64::from_str_radix(&s[2..], 8).map_or(f64::NAN, |v| v as f64),
        ));
    }
    if re(r"^0x[0-9a-fA-F]+$") {
        return Scalar::Num(js_number_string(
            i64::from_str_radix(&s[2..], 16).map_or(f64::NAN, |v| v as f64),
        ));
    }
    if re(r"^[-+]?[0-9]+$") || re(r"^[-+]?(?:\.[0-9]+|[0-9]+(?:\.[0-9]*)?)(?:[eE][-+]?[0-9]+)?$") {
        return Scalar::Num(js_number_string(s.parse().unwrap_or(f64::NAN)));
    }
    if re(r"^[-+]?\.(?:inf|Inf|INF)$") {
        return Scalar::Num(
            if s.starts_with('-') {
                "-Infinity"
            } else {
                "Infinity"
            }
            .into(),
        );
    }
    if re(r"^\.(?:nan|NaN|NAN)$") {
        return Scalar::Num("NaN".into());
    }
    Scalar::Str(s.to_owned())
}

impl<'a> Yaml<'a> {
    fn parse(text: &'a str) -> Result<Option<Node>, String> {
        let lines = text
            .lines()
            .filter_map(|raw| {
                let text = raw.trim_start_matches(' ');
                let text = text.trim_end();
                (!text.is_empty() && !text.starts_with('#')).then(|| Line {
                    indent: raw.len() - raw.trim_start_matches(' ').len(),
                    text,
                })
            })
            .collect();
        let mut y = Yaml { lines, pos: 0 };
        if y.lines.is_empty() {
            return Ok(None);
        }
        let first = y.lines[0].indent;
        let node = if is_seq_entry(y.lines[0].text) {
            y.seq(first)?
        } else {
            let text = y.lines[0].text;
            y.pos += 1;
            y.inline(text, first, None)?
        };
        if y.pos < y.lines.len() {
            return Err(format!("Unexpected content at line {}", y.pos + 1));
        }
        Ok(Some(node))
    }

    fn peek(&self) -> Option<&Line<'a>> {
        self.lines.get(self.pos)
    }

    /// A block sequence whose `-` entries are at `indent`.
    fn seq(&mut self, indent: usize) -> Result<Node, String> {
        let mut items = Vec::new();
        while let Some(line) = self.peek() {
            if line.indent != indent || !is_seq_entry(line.text) {
                break;
            }
            let rest = line.text[1..].trim_start_matches(' ');
            let col = indent + (line.text.len() - rest.len());
            self.pos += 1;
            if rest.is_empty() || rest.starts_with('#') {
                items.push(self.nested(indent as isize, None)?);
            } else {
                items.push(self.inline(rest, col, Some(indent))?);
            }
        }
        Ok(Node::Seq(items))
    }

    /// The block under a line (more indented, or a sequence at `key_col`).
    fn nested(&mut self, parent: isize, key_col: Option<usize>) -> Result<Node, String> {
        let Some(line) = self.peek() else {
            return Ok(Node::Scalar(Scalar::Null));
        };
        let (indent, text) = (line.indent, line.text);
        if indent as isize > parent {
            if is_seq_entry(text) {
                return self.seq(indent);
            }
            self.pos += 1;
            return self.inline(text, indent, None);
        }
        if key_col == Some(indent) && is_seq_entry(text) {
            return self.seq(indent);
        }
        Ok(Node::Scalar(Scalar::Null))
    }

    /// A node starting inline at column `col`: a mapping (whose further keys
    /// are at `col`) or a scalar. `seq_indent` is the enclosing `-` column.
    fn inline(
        &mut self,
        text: &'a str,
        col: usize,
        seq_indent: Option<usize>,
    ) -> Result<Node, String> {
        if is_seq_entry(text) {
            return Err("Compact nested sequences are not supported".into());
        }
        let parent = seq_indent.map_or(col as isize - 1, |i| i as isize);
        let Some((key, rest)) = self.split_key(text)? else {
            return Ok(Node::Scalar(self.scalar(text, parent)?));
        };
        let mut entries = Vec::new();
        let value = self.value(rest, col)?;
        entries.push((Node::Scalar(key), value));
        while let Some(line) = self.peek() {
            if line.indent != col || is_seq_entry(line.text) {
                break;
            }
            let text = line.text;
            self.pos += 1;
            let Some((key, rest)) = self.split_key(text)? else {
                return Err("Implicit keys need to be followed by map values".into());
            };
            let value = self.value(rest, col)?;
            entries.push((Node::Scalar(key), value));
        }
        Ok(Node::Map(entries))
    }

    fn value(&mut self, rest: &'a str, key_col: usize) -> Result<Node, String> {
        let rest = rest.trim_start();
        if rest.is_empty() || rest.starts_with('#') {
            return self.nested(key_col as isize, Some(key_col));
        }
        if !rest.starts_with(['"', '\'']) && plain_key_end(rest).is_some() {
            return Err("Nested mappings are not allowed in compact mappings".into());
        }
        Ok(Node::Scalar(self.scalar(rest, key_col as isize)?))
    }

    /// Splits `key: rest` when `text` is a mapping entry.
    fn split_key(&self, text: &'a str) -> Result<Option<(Scalar, &'a str)>, String> {
        if let Some(q) = text.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let Some((value, used)) = quoted_line(text, q) else {
                return Ok(None);
            };
            let after = text[used..].trim_start();
            if let Some(rest) = after.strip_prefix(':')
                && (rest.is_empty() || rest.starts_with(' '))
            {
                return Ok(Some((Scalar::Str(value), rest)));
            }
            return Ok(None);
        }
        Ok(plain_key_end(text).map(|i| (resolve_plain(text[..i].trim_end()), &text[i + 1..])))
    }

    /// A scalar starting with `text`, continued on lines indented past `parent`.
    fn scalar(&mut self, text: &'a str, parent: isize) -> Result<Scalar, String> {
        let mut continuation = Vec::new();
        while let Some(line) = self.peek() {
            if line.indent as isize <= parent {
                break;
            }
            continuation.push(line.text);
            self.pos += 1;
        }
        match text.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let mut full = text.to_owned();
                for c in &continuation {
                    full.push('\n');
                    full.push_str(c);
                }
                let Some((value, used)) = quoted(&full, q) else {
                    return Err(if q == '"' {
                        "Missing closing \"quote".into()
                    } else {
                        "Missing closing 'quote".into()
                    });
                };
                let rest = full[used..].trim_start();
                if !rest.is_empty() && !rest.starts_with('#') {
                    return Err("Unexpected scalar at node end".into());
                }
                Ok(Scalar::Str(value))
            }
            Some('[' | '{') => Err("Flow collections are not supported".into()),
            Some('|' | '>' | '&' | '*' | '!' | '%' | '@' | '`') => {
                Err(format!("Unsupported YAML syntax: {text}"))
            }
            _ => {
                let mut value = strip_comment(text).trim_end().to_owned();
                for c in continuation {
                    let c = strip_comment(c).trim();
                    if !c.is_empty() {
                        value.push(' ');
                        value.push_str(c);
                    }
                }
                if value.is_empty() {
                    return Ok(Scalar::Null);
                }
                Ok(resolve_plain(&value))
            }
        }
    }
}

/// A quoted scalar on one line: its value and the bytes used.
fn quoted_line(text: &str, q: char) -> Option<(String, usize)> {
    let line = text.split('\n').next().unwrap_or("");
    quoted(line, q)
}

/// Parses a quoted scalar (which may span lines, folded like YAML).
fn quoted(text: &str, q: char) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut chars = text.char_indices().skip(1).peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\'' if q == '\'' => {
                if chars.peek().map(|(_, c)| *c) == Some('\'') {
                    chars.next();
                    out.push('\'');
                } else {
                    return Some((out, i + 1));
                }
            }
            '"' if q == '"' => return Some((out, i + 1)),
            '\\' if q == '"' => {
                let (_, e) = chars.next()?;
                match e {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '0' => out.push('\0'),
                    ' ' => out.push(' '),
                    '/' => out.push('/'),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'x' | 'u' | 'U' => {
                        let n = match e {
                            'x' => 2,
                            'u' => 4,
                            _ => 8,
                        };
                        let hex: String = (0..n)
                            .filter_map(|_| chars.next().map(|(_, c)| c))
                            .collect();
                        out.push(
                            u32::from_str_radix(&hex, 16)
                                .ok()
                                .and_then(char::from_u32)?,
                        );
                    }
                    '\n' => {
                        while chars.peek().is_some_and(|(_, c)| *c == ' ' || *c == '\t') {
                            chars.next();
                        }
                    }
                    other => {
                        out.push('\\');
                        out.push(other);
                    }
                }
            }
            '\n' => {
                // Line folding: trailing spaces dropped, a single break becomes
                // a space, each empty line a newline.
                while out.ends_with([' ', '\t']) {
                    out.pop();
                }
                let mut breaks = 0;
                loop {
                    while chars.peek().is_some_and(|(_, c)| *c == ' ' || *c == '\t') {
                        chars.next();
                    }
                    if chars.peek().is_some_and(|(_, c)| *c == '\n') {
                        chars.next();
                        breaks += 1;
                    } else {
                        break;
                    }
                }
                if breaks == 0 {
                    out.push(' ');
                } else {
                    out.extend(std::iter::repeat_n('\n', breaks));
                }
            }
            c => out.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inputs and the results of Playwright 1.63's `parseAriaSnapshot`.
    const CASES: &str = include_str!("data/aria_template_cases.json");

    #[test]
    fn parses_like_playwright() {
        let cases: Vec<Value> = serde_json::from_str(CASES).unwrap();
        for case in cases {
            let input = case["input"].as_str().unwrap();
            let got = parse_aria_template(input);
            match case["errors"].as_array().and_then(|e| e.first()) {
                Some(err) => assert_eq!(got, Err(err.as_str().unwrap().to_owned()), "{input}"),
                None => assert_eq!(got, Ok(case["fragment"].clone()), "{input}"),
            }
        }
    }

    #[test]
    fn unshift_removes_common_indent_and_blank_lines() {
        assert_eq!(
            unshift("\n      - list:\n        - listitem: a\n\n      - button\n    "),
            "- list:\n  - listitem: a\n- button"
        );
    }
}
