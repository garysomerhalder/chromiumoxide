//! Firefox: launch arguments and WebDriver BiDi value conversion (pure).

use std::path::Path;

use serde_json::{Map, Number, Value};

/// Playwright's Firefox arguments with the WebDriver BiDi endpoint instead
/// of Juggler's pipe. Playwright passes `-silent` (no window) because its
/// Juggler agent keeps Firefox alive; over BiDi Firefox quits without a
/// window, so an `about:blank` window is opened instead.
pub fn command_line(profile: &Path, headless: bool, args: &[String]) -> Vec<String> {
    let mut out = vec!["-no-remote".to_owned()];
    if headless {
        out.push("-headless".to_owned());
    } else {
        out.push("-wait-for-browser".to_owned());
        out.push("-foreground".to_owned());
    }
    out.push("-profile".to_owned());
    out.push(profile.display().to_string());
    out.push("--remote-debugging-port=0".to_owned());
    out.extend(args.iter().cloned());
    out.push("about:blank".to_owned());
    out
}

/// A BiDi `script.RemoteValue` as JSON, the way Playwright's `evaluate`
/// returns values: `undefined` is `null`, special numbers become `null`,
/// dates their ISO string, maps and objects objects, sets and arrays arrays,
/// other platform objects `{}`.
pub fn remote_value_to_json(v: &Value) -> Value {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
    let value = v.get("value");
    match kind {
        "undefined" | "null" => Value::Null,
        "string" => value.cloned().unwrap_or(Value::Null),
        "boolean" => value.cloned().unwrap_or(Value::Null),
        "number" => match value {
            Some(Value::Number(n)) => Value::Number(n.clone()),
            Some(Value::String(s)) if s == "-0" => Value::Number(Number::from(0)),
            _ => Value::Null,
        },
        "bigint" => value
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<i64>().ok())
            .map(|n| Value::Number(n.into()))
            .unwrap_or(Value::Null),
        "date" => value.cloned().unwrap_or(Value::Null),
        "array" | "set" => Value::Array(
            value
                .and_then(Value::as_array)
                .map(|items| items.iter().map(remote_value_to_json).collect())
                .unwrap_or_default(),
        ),
        "object" | "map" => {
            let mut map = Map::new();
            for pair in value.and_then(Value::as_array).into_iter().flatten() {
                let (Some(k), Some(val)) = (pair.get(0), pair.get(1)) else {
                    continue;
                };
                let key = match k {
                    Value::String(s) => s.clone(),
                    other => remote_value_to_json(other).to_string(),
                };
                map.insert(key, remote_value_to_json(val));
            }
            Value::Object(map)
        }
        _ => Value::Object(Map::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn playwright_arguments_with_the_bidi_port() {
        let args = command_line(Path::new("/tmp/p"), true, &["-foo".to_owned()]);
        assert_eq!(
            args,
            [
                "-no-remote",
                "-headless",
                "-profile",
                "/tmp/p",
                "--remote-debugging-port=0",
                "-foo",
                "about:blank"
            ]
        );
        let headed = command_line(Path::new("/tmp/p"), false, &[]);
        assert_eq!(&headed[1..3], ["-wait-for-browser", "-foreground"]);
    }

    #[test]
    fn remote_values_become_json() {
        let v = json!({"type": "object", "value": [
            ["n", {"type": "number", "value": 1.5}],
            ["s", {"type": "string", "value": "x"}],
            ["u", {"type": "undefined"}],
            ["nan", {"type": "number", "value": "NaN"}],
            ["a", {"type": "array", "value": [{"type": "boolean", "value": true}, {"type": "null"}]}],
            ["d", {"type": "date", "value": "2026-11-23T00:00:00.000Z"}],
            ["el", {"type": "node", "sharedId": "x"}]
        ]});
        assert_eq!(
            remote_value_to_json(&v),
            json!({"n": 1.5, "s": "x", "u": null, "nan": null, "a": [true, null],
                   "d": "2026-11-23T00:00:00.000Z", "el": {}})
        );
        assert_eq!(
            remote_value_to_json(&json!({"type": "bigint", "value": "42"})),
            json!(42)
        );
    }
}
