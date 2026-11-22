//! Console messages and page errors as Playwright renders them
//! (`renderPreview`, `sparseArrayToString`, `exceptionToError`).
//!
//! Works on protocol JSON values (`Runtime.RemoteObject`, `ExceptionDetails`)
//! without any I/O.

use serde_json::Value;

/// Playwright's `renderPreview` for one console argument.
pub fn render_preview(_object: &Value) -> String {
    unimplemented!()
}

/// `ConsoleMessage.text()`: previews joined by spaces (nodes are `JSHandle@node`).
pub fn console_text(_args: &[Value]) -> String {
    unimplemented!()
}

/// Source location of a console call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Location {
    pub url: String,
    pub line_number: u32,
    pub column_number: u32,
}

/// The first frame of a `Runtime.StackTrace`.
pub fn location(_stack_trace: Option<&Value>) -> Location {
    unimplemented!()
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
pub fn exception_to_error(_details: &Value) -> PageError {
    unimplemented!()
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
