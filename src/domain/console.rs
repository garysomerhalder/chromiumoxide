//! Console messages and page errors as Playwright renders them
//! (`renderPreview`, `sparseArrayToString`, `exceptionToError`).
//!
//! Works on protocol JSON values (`Runtime.RemoteObject`, `ExceptionDetails`)
//! without any I/O.

use serde_json::Value;

/// Playwright's `renderPreview` for one console argument.
pub fn render_preview(object: &Value) -> String {
    if object.get("type").and_then(Value::as_str) == Some("undefined") {
        return "undefined".into();
    }
    if let Some(v) = object.get("value") {
        return js_string(v);
    }
    if let Some(u) = object.get("unserializableValue").and_then(Value::as_str) {
        return u.to_owned();
    }
    let description = object
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let props = object
        .pointer("/preview/properties")
        .and_then(Value::as_array);
    if let Some(props) = props {
        if description == "Object" {
            let tokens: Vec<String> = props
                .iter()
                .map(|p| format!("{}: {}", str_of(p, "name"), str_of(p, "value")))
                .collect();
            return format!("{{{}}}", tokens.join(", "));
        }
        if object.get("subtype").and_then(Value::as_str) == Some("array") {
            return sparse_array_to_string(props);
        }
    }
    description.to_owned()
}

fn str_of(v: &Value, k: &str) -> String {
    v.get(k)
        .map(js_string)
        .unwrap_or_else(|| "undefined".into())
}

/// JavaScript's `String(value)` for JSON values.
fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// Playwright's `sparseArrayToString`.
fn sparse_array_to_string(entries: &[Value]) -> String {
    let mut items: Vec<(i64, String)> = entries
        .iter()
        .filter_map(|e| {
            Some((
                str_of(e, "name").parse::<i64>().ok().filter(|i| *i >= 0)?,
                str_of(e, "value"),
            ))
        })
        .collect();
    items.sort_by_key(|(i, _)| *i);
    let mut tokens = Vec::new();
    let mut last = -1;
    for (index, value) in items {
        match index - last - 1 {
            0 => {}
            1 => tokens.push("empty".to_owned()),
            n => tokens.push(format!("empty x {n}")),
        }
        tokens.push(value);
        last = index;
    }
    format!("[{}]", tokens.join(", "))
}

/// `ConsoleMessage.text()`: previews joined by spaces (nodes are `JSHandle@node`).
pub fn console_text(args: &[Value]) -> String {
    args.iter()
        .map(|a| {
            if a.get("subtype").and_then(Value::as_str) == Some("node") {
                "JSHandle@node".to_owned()
            } else {
                render_preview(a)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Source location of a console call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Location {
    pub url: String,
    pub line_number: u32,
    pub column_number: u32,
}

/// The first frame of a `Runtime.StackTrace`.
pub fn location(stack_trace: Option<&Value>) -> Location {
    let Some(frame) = stack_trace.and_then(|s| s.pointer("/callFrames/0")) else {
        return Location::default();
    };
    Location {
        url: frame
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        line_number: frame
            .get("lineNumber")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32,
        column_number: frame
            .get("columnNumber")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32,
    }
}

/// An uncaught exception (`page.on('pageerror')`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageError {
    pub name: String,
    pub message: String,
    /// Full `name: message\n    at ...` text, empty without frames.
    pub stack: String,
}

/// Playwright's `exceptionToError`.
pub fn exception_to_error(details: &Value) -> PageError {
    let with_stack = match details.get("exception") {
        Some(e) => e
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| js_string(e.get("value").unwrap_or(&Value::Null))),
        None => {
            let mut m = details
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            for f in details
                .pointer("/stackTrace/callFrames")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let name = f
                    .get("functionName")
                    .and_then(Value::as_str)
                    .filter(|n| !n.is_empty())
                    .unwrap_or("<anonymous>");
                m.push_str(&format!(
                    "\n    at {name} ({}:{}:{})",
                    str_of(f, "url"),
                    str_of(f, "lineNumber"),
                    str_of(f, "columnNumber")
                ));
            }
            m
        }
    };
    let lines: Vec<&str> = with_stack.split('\n').collect();
    let (with_name, stack) = match lines.iter().position(|l| l.starts_with("    at")) {
        Some(i) => (lines[..i].join("\n"), with_stack.clone()),
        None => (with_stack.clone(), String::new()),
    };
    let (name, message) = match with_name.find(':') {
        Some(i) => {
            let message = if i + 2 <= with_name.len() {
                with_name[i + 2..].to_owned()
            } else {
                with_name.clone()
            };
            (with_name[..i].to_owned(), message)
        }
        None => (String::new(), with_name.clone()),
    };
    let name_override = details
        .pointer("/exception/preview/properties")
        .and_then(Value::as_array)
        .and_then(|ps| {
            ps.iter()
                .find(|p| p.get("name").and_then(Value::as_str) == Some("name"))
        })
        .map(|p| {
            p.get("value")
                .and_then(Value::as_str)
                .unwrap_or("Error")
                .to_owned()
        });
    PageError {
        name: name_override.unwrap_or(name),
        message,
        stack,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn primitives_objects_arrays_and_nodes_like_playwright() {
        let args = vec![
            json!({"type": "string", "value": "hello"}),
            json!({"type": "number", "value": 42, "description": "42"}),
            json!({"type": "boolean", "value": true}),
            json!({"type": "object", "subtype": "null", "value": null}),
            json!({"type": "undefined"}),
            json!({"type": "object", "className": "Object", "description": "Object", "objectId": "1",
                   "preview": {"type": "object", "description": "Object", "overflow": false,
                               "properties": [{"name": "a", "type": "number", "value": "1"}, {"name": "b", "type": "object", "value": "Array(2)", "subtype": "array"}]}}),
            json!({"type": "object", "subtype": "array", "className": "Array", "description": "Array(2)", "objectId": "2",
                   "preview": {"type": "object", "subtype": "array", "description": "Array(2)", "overflow": false,
                               "properties": [{"name": "0", "type": "number", "value": "1"}, {"name": "1", "type": "string", "value": "x"}]}}),
            json!({"type": "object", "subtype": "node", "className": "HTMLBodyElement", "description": "body", "objectId": "3"}),
        ];
        // Recorded from Playwright 1.63: console.log('hello', 42, true, null, undefined, {a: 1, b: [1,2]}, [1, 'x'], document.body)
        assert_eq!(
            console_text(&args),
            "hello 42 true null undefined {a: 1, b: Array(2)} [1, x] JSHandle@node"
        );
    }

    #[test]
    fn sparse_arrays_and_special_numbers() {
        let sparse = json!({"type": "object", "subtype": "array", "description": "Array(5)",
            "preview": {"properties": [{"name": "0", "value": "1"}, {"name": "2", "value": "3"}, {"name": "4", "value": "5"}, {"name": "length", "value": "5"}]}});
        assert_eq!(render_preview(&sparse), "[1, empty, 3, empty, 5]");
        let gap = json!({"type": "object", "subtype": "array", "description": "Array(4)",
            "preview": {"properties": [{"name": "3", "value": "z"}]}});
        assert_eq!(render_preview(&gap), "[empty x 3, z]");
        assert_eq!(
            render_preview(
                &json!({"type": "number", "unserializableValue": "NaN", "description": "NaN"})
            ),
            "NaN"
        );
        assert_eq!(
            render_preview(&json!({"type": "function", "description": "() => 1", "objectId": "9"})),
            "() => 1"
        );
    }

    #[test]
    fn location_from_stack() {
        let st = json!({"callFrames": [{"url": "http://a/x.js", "lineNumber": 3, "columnNumber": 7, "functionName": "f"}]});
        assert_eq!(
            location(Some(&st)),
            Location {
                url: "http://a/x.js".into(),
                line_number: 3,
                column_number: 7
            }
        );
        assert_eq!(location(None), Location::default());
    }

    #[test]
    fn exceptions_like_playwright() {
        let d = json!({"text": "Uncaught", "exception": {"type": "object", "subtype": "error", "className": "Error",
            "description": "Error: boom\n    at <anonymous>:1:28"}});
        let e = exception_to_error(&d);
        assert_eq!((e.name.as_str(), e.message.as_str()), ("Error", "boom"));
        assert_eq!(e.stack, "Error: boom\n    at <anonymous>:1:28");
        let d = json!({"text": "Uncaught", "exception": {"type": "object", "description": "TypeError: x is not a function",
            "preview": {"properties": [{"name": "name", "value": "MyError"}]}}});
        let e = exception_to_error(&d);
        assert_eq!(
            (e.name.as_str(), e.message.as_str(), e.stack.as_str()),
            ("MyError", "x is not a function", "")
        );
        let d = json!({"text": "Uncaught", "exception": {"type": "string", "value": "plain"}});
        assert_eq!(exception_to_error(&d).message, "plain");
    }
}
