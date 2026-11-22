//! Turning user JavaScript into protocol calls, and remote values back.

use serde_json::{Value, json};

use crate::domain::error::{Error, Result};

/// Wraps an expression or function source so that functions are called with
/// the argument and expressions are returned (Playwright's semantics for
/// string arguments to `evaluate`).
pub fn function_declaration(source: &str) -> String {
    let source = source.trim().trim_end_matches(';');
    format!(
        "async function(__chromiumoxide_arg) {{ const __v = (\n{source}\n); return typeof __v === 'function' ? await __v(__chromiumoxide_arg) : await __v; }}"
    )
}

/// `Runtime.callFunctionOn` parameters for a function call in a context.
pub fn call_params(declaration: &str, context_id: i64, args: &[Value]) -> Value {
    let arguments: Vec<Value> = args.iter().map(|v| json!({"value": v})).collect();
    json!({
        "functionDeclaration": declaration,
        "executionContextId": context_id,
        "arguments": arguments,
        "returnByValue": true,
        "awaitPromise": true,
        "userGesture": true,
    })
}

/// Extracts the value of a `callFunctionOn`/`evaluate` response.
pub fn result_value(response: &Value) -> Result<Value> {
    if let Some(details) = response.get("exceptionDetails") {
        let message = details
            .pointer("/exception/description")
            .and_then(Value::as_str)
            .or_else(|| details.pointer("/exception/value").and_then(Value::as_str))
            .or_else(|| details.get("text").and_then(Value::as_str))
            .unwrap_or("unknown error");
        return Err(Error::Evaluation(message.to_owned()));
    }
    let result = response.get("result").cloned().unwrap_or(Value::Null);
    if let Some(u) = result.get("unserializableValue").and_then(Value::as_str) {
        return Ok(match u {
            "-0" => json!(0),
            s if s.ends_with('n') => s[..s.len() - 1]
                .parse::<i64>()
                .map(Value::from)
                .unwrap_or(Value::Null),
            _ => Value::Null,
        });
    }
    Ok(result.get("value").cloned().unwrap_or(Value::Null))
}

/// True for errors that mean "the context went away, retry in the new one".
pub fn is_context_lost(e: &Error) -> bool {
    match e {
        Error::Protocol { message, .. } => {
            message.contains("Cannot find context")
                || message.contains("Execution context was destroyed")
                || message.contains("Inspected target navigated or closed")
        }
        Error::Evaluation(m) => m.contains("Execution context was destroyed"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exceptions_become_evaluation_errors() {
        let r = json!({"result": {"type": "object"}, "exceptionDetails": {"text": "Uncaught", "exception": {"description": "Error: boom\n    at x"}}});
        assert!(
            matches!(result_value(&r), Err(Error::Evaluation(m)) if m.starts_with("Error: boom"))
        );
    }

    #[test]
    fn values_and_special_numbers() {
        assert_eq!(
            result_value(&json!({"result": {"type": "number", "value": 3}})).unwrap(),
            json!(3)
        );
        assert_eq!(
            result_value(&json!({"result": {"type": "undefined"}})).unwrap(),
            Value::Null
        );
        assert_eq!(
            result_value(&json!({"result": {"type": "number", "unserializableValue": "-0"}}))
                .unwrap(),
            json!(0)
        );
        assert_eq!(
            result_value(&json!({"result": {"type": "bigint", "unserializableValue": "12n"}}))
                .unwrap(),
            json!(12)
        );
    }

    #[test]
    fn declaration_wraps_functions_and_expressions() {
        let d = function_declaration("1 + 1;");
        assert!(d.contains("(\n1 + 1\n)"));
    }
}
