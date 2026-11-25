//! Reader for JSON protocol descriptions: WebKit's inspector protocol
//! (`protocol.json`, as Playwright's WebKit build ships it) has the same
//! shape as Chrome's `browser_protocol.json`. It is read into the model the
//! PDL parser makes, so one generator serves both.

use std::borrow::Cow;

use serde_json::Value;

use crate::pdl::dep::is_circular_dep;
use crate::pdl::error::Error;
use crate::pdl::*;

fn text(v: &Value, key: &str) -> Option<Cow<'static, str>> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| Cow::Owned(s.to_owned()))
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn list<'v>(v: &'v Value, key: &str) -> &'v [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn name(v: &Value, key: &str, what: &str) -> Result<Cow<'static, str>, Error> {
    text(v, key).ok_or_else(|| Error {
        message: format!("{what} without a `{key}`: {v}"),
    })
}

fn variants(v: &Value) -> Vec<Variant<'static>> {
    list(v, "enum")
        .iter()
        .filter_map(Value::as_str)
        .map(|s| Variant {
            description: None,
            name: Cow::Owned(s.to_owned()),
        })
        .collect()
}

/// The type of a member, a type's `extends`, or an array's `items`.
fn ty(v: &Value) -> Type<'static> {
    if let Some(r) = v.get("$ref").and_then(Value::as_str) {
        // WebKit's own protocol has `"$ref": "boolean"` once.
        return match r {
            "integer" | "number" | "boolean" | "string" | "object" | "any" => {
                Type::new(r, false).into_owned()
            }
            _ => Type::Ref(Cow::Owned(r.to_owned())),
        };
    }
    match v.get("type").and_then(Value::as_str).unwrap_or("any") {
        "array" => Type::ArrayOf(Box::new(v.get("items").map_or(Type::Any, ty))),
        "string" if v.get("enum").is_some() => Type::Enum(variants(v)),
        other => Type::new(other, false).into_owned(),
    }
}

impl Type<'_> {
    fn into_owned(self) -> Type<'static> {
        match self {
            Type::Integer => Type::Integer,
            Type::Number => Type::Number,
            Type::Boolean => Type::Boolean,
            Type::String => Type::String,
            Type::Object => Type::Object,
            Type::Any => Type::Any,
            Type::Binary => Type::Binary,
            Type::Enum(v) => Type::Enum(
                v.into_iter()
                    .map(|v| Variant {
                        description: v.description.map(|d| Cow::Owned(d.into_owned())),
                        name: Cow::Owned(v.name.into_owned()),
                    })
                    .collect(),
            ),
            Type::ArrayOf(t) => Type::ArrayOf(Box::new(t.into_owned())),
            Type::Ref(r) => Type::Ref(Cow::Owned(r.into_owned())),
        }
    }
}

fn params(domain: &str, v: &Value, key: &str) -> Result<Vec<Param<'static>>, Error> {
    list(v, key)
        .iter()
        .map(|p| {
            let n = name(p, "name", "member")?;
            Ok(Param {
                description: text(p, "description"),
                experimental: flag(p, "experimental"),
                deprecated: flag(p, "deprecated"),
                optional: flag(p, "optional"),
                r#type: ty(p),
                raw_name: Cow::Owned(format!("{domain}.{n}")),
                is_circular_dep: is_circular_dep(domain, &n),
                name: n,
            })
        })
        .collect()
}

/// Joins another variant's members into `into`: a member missing from
/// either side becomes optional.
fn merge(into: &mut Vec<Param<'static>>, other: Vec<Param<'static>>) {
    for p in into.iter_mut() {
        if !other.iter().any(|o| o.name == p.name) {
            p.optional = true;
        }
    }
    for mut o in other {
        if !into.iter().any(|p| p.name == o.name) {
            o.optional = true;
            into.push(o);
        }
    }
}

/// Reads a JSON protocol description: an array of domains, or an object
/// with a `domains` array (and an optional `version`).
pub fn parse_protocol_json(input: &str) -> Result<Protocol<'static>, Error> {
    let root: Value = serde_json::from_str(input).map_err(|e| Error {
        message: format!("invalid protocol JSON: {e}"),
    })?;
    let domains = match &root {
        Value::Array(a) => a.as_slice(),
        other => list(other, "domains"),
    };
    let mut protocol = Protocol {
        version: Version {
            major: root["version"]["major"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1),
            minor: root["version"]["minor"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        },
        ..Default::default()
    };
    for d in domains {
        let dn = name(d, "domain", "domain")?;
        let mut domain = Domain {
            description: text(d, "description"),
            experimental: flag(d, "experimental"),
            deprecated: flag(d, "deprecated"),
            name: dn.clone(),
            dependencies: list(d, "dependencies")
                .iter()
                .filter_map(Value::as_str)
                .map(|s| Cow::Owned(s.to_owned()))
                .collect(),
            types: vec![],
            commands: vec![],
            events: vec![],
        };
        for t in list(d, "types") {
            let n = name(t, "id", "type")?;
            if domain.types.iter().any(|x| x.name == n) {
                continue;
            }
            let extends = match ty(t) {
                Type::Enum(_) => Type::String,
                other => other,
            };
            let item = if t.get("enum").is_some() {
                Some(Item::Enum(variants(t)))
            } else if t.get("properties").is_some() {
                Some(Item::Properties(params(&dn, t, "properties")?))
            } else {
                None
            };
            domain.types.push(TypeDef {
                description: text(t, "description"),
                experimental: flag(t, "experimental"),
                deprecated: flag(t, "deprecated"),
                raw_name: Cow::Owned(format!("{dn}.{n}")),
                is_circular_dep: is_circular_dep(&dn, &n),
                name: n,
                extends,
                item,
            });
        }
        for c in list(d, "commands") {
            let n = name(c, "name", "command")?;
            let parameters = params(&dn, c, "parameters")?;
            // A command listed once per platform (`condition`): one command
            // with every variant's parameters, optional unless all have it.
            if let Some(seen) = domain.commands.iter_mut().find(|x| x.name == n) {
                merge(&mut seen.parameters, parameters);
                merge(&mut seen.returns, params(&dn, c, "returns")?);
                continue;
            }
            domain.commands.push(Command {
                description: text(c, "description"),
                experimental: flag(c, "experimental"),
                deprecated: flag(c, "deprecated"),
                redirect: None,
                parameters,
                returns: params(&dn, c, "returns")?,
                raw_name: Cow::Owned(format!("{dn}.{n}")),
                is_circular_dep: is_circular_dep(&dn, &n),
                name: n,
            });
        }
        for e in list(d, "events") {
            let n = name(e, "name", "event")?;
            if let Some(seen) = domain.events.iter_mut().find(|x| x.name == n) {
                merge(&mut seen.parameters, params(&dn, e, "parameters")?);
                continue;
            }
            domain.events.push(Event {
                description: text(e, "description"),
                experimental: flag(e, "experimental"),
                deprecated: flag(e, "deprecated"),
                parameters: params(&dn, e, "parameters")?,
                raw_name: Cow::Owned(format!("{dn}.{n}")),
                is_circular_dep: is_circular_dep(&dn, &n),
                name: n,
            });
        }
        protocol.domains.push(domain);
    }
    Ok(protocol)
}
