//! WebKit's inspector protocol (`protocol.json`) read into the same model
//! as Chrome's PDL.

use chromiumoxide_pdl::pdl::{Item, Type, parse_protocol_json};

const JSON: &str = r#"[
  {"domain": "Page", "description": "Pages.",
   "types": [
     {"id": "Appearance", "type": "string", "enum": ["Light", "Dark"]},
     {"id": "Frame", "type": "object", "description": "A frame.",
      "properties": [
        {"name": "id", "$ref": "Network.FrameId"},
        {"name": "url", "type": "string"},
        {"name": "children", "type": "array", "items": {"$ref": "Frame"}, "optional": true},
        {"name": "kind", "type": "string", "enum": ["a", "b"]}
      ]},
     {"id": "Ids", "type": "array", "items": {"type": "integer"}}
   ],
   "commands": [
     {"name": "navigate", "description": "Goes.",
      "parameters": [{"name": "url", "type": "string"}],
      "returns": [{"name": "frameId", "$ref": "Network.FrameId", "optional": true}]}
   ],
   "events": [
     {"name": "loadEventFired", "parameters": [{"name": "timestamp", "type": "number"}]}
   ]}
]"#;

#[test]
fn webkit_protocol_json_reads_into_the_pdl_model() {
    let p = parse_protocol_json(JSON).unwrap();
    assert_eq!(p.domains.len(), 1);
    let d = &p.domains[0];
    assert_eq!(d.name, "Page");
    assert_eq!(d.description.as_deref(), Some("Pages."));

    let appearance = &d.types[0];
    assert_eq!(appearance.raw_name, "Page.Appearance");
    assert_eq!(appearance.extends, Type::String);
    let Some(Item::Enum(vars)) = &appearance.item else {
        panic!("{appearance:?}")
    };
    assert_eq!(vars.iter().map(|v| &*v.name).collect::<Vec<_>>(), ["Light", "Dark"]);

    let frame = &d.types[1];
    assert_eq!(frame.extends, Type::Object);
    let Some(Item::Properties(props)) = &frame.item else {
        panic!("{frame:?}")
    };
    assert_eq!(props[0].r#type, Type::Ref("Network.FrameId".into()));
    assert_eq!(props[1].r#type, Type::String);
    assert!(props[2].optional);
    assert_eq!(
        props[2].r#type,
        Type::ArrayOf(Box::new(Type::Ref("Frame".into())))
    );
    let Type::Enum(kinds) = &props[3].r#type else {
        panic!("{:?}", props[3])
    };
    assert_eq!(kinds.len(), 2);
    assert_eq!(d.types[2].extends, Type::ArrayOf(Box::new(Type::Integer)));

    let nav = &d.commands[0];
    assert_eq!(nav.raw_name, "Page.navigate");
    assert_eq!(nav.parameters[0].name, "url");
    assert!(nav.returns[0].optional);
    assert_eq!(d.events[0].name, "loadEventFired");
    assert_eq!(d.events[0].parameters[0].r#type, Type::Number);
}
