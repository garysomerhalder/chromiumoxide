//! Keyboard and mouse models (Playwright's `input.ts` + US keyboard layout).
//!
//! Pure state machines: they turn API calls into the raw events a page must
//! receive; adapters only send them.

use std::collections::{BTreeSet, HashMap};

/// A key as the browser should see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyDescription {
    pub key: String,
    pub key_code: u32,
    pub key_code_without_location: u32,
    pub code: String,
    pub text: String,
    pub location: u32,
    pub shifted: Option<Box<KeyDescription>>,
}

/// Which raw key event to dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventType {
    /// Produces text.
    KeyDown,
    /// No text (modifiers held, non-printable keys).
    RawKeyDown,
    KeyUp,
}

/// One raw key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub kind: KeyEventType,
    pub modifiers: u32,
    pub windows_virtual_key_code: u32,
    pub code: String,
    pub key: String,
    pub text: String,
    pub auto_repeat: bool,
    pub location: u32,
    pub is_keypad: bool,
}

/// Modifier bit masks (CDP `modifiers`).
pub const ALT: u32 = 1;
pub const CONTROL: u32 = 2;
pub const META: u32 = 4;
pub const SHIFT: u32 = 8;

/// The modifier keys, in Playwright's order.
pub const MODIFIERS: [&str; 4] = ["Alt", "Control", "Meta", "Shift"];

fn layout() -> &'static Layout {
    static LAYOUT: std::sync::OnceLock<Layout> = std::sync::OnceLock::new();
    LAYOUT.get_or_init(|| {
        let raw: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(include_str!("data/us_keyboard_layout.json"))
                .expect("valid layout");
        build_layout(&raw)
    })
}

/// Playwright's `buildLayoutClosure`.
fn build_layout(raw: &serde_json::Map<String, serde_json::Value>) -> Layout {
    let aliases: &[(&str, &[&str])] = &[
        ("ShiftLeft", &["Shift"]),
        ("ControlLeft", &["Control"]),
        ("AltLeft", &["Alt"]),
        ("MetaLeft", &["Meta"]),
        ("Enter", &["\n", "\r"]),
    ];
    let mut out = Layout::new();
    for (code, def) in raw {
        let str_of = |k: &str| {
            def.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        let num_of = |k: &str| def.get(k).and_then(|v| v.as_u64()).map(|v| v as u32);
        let key = str_of("key");
        let key_code = num_of("keyCode").unwrap_or(0);
        let mut d = KeyDescription {
            key: key.clone(),
            key_code,
            key_code_without_location: num_of("keyCodeWithoutLocation").unwrap_or(key_code),
            code: code.clone(),
            text: str_of("text"),
            location: num_of("location").unwrap_or(0),
            shifted: None,
        };
        if key.chars().count() == 1 {
            d.text = key.clone();
        }
        let shifted = def
            .get("shiftKey")
            .and_then(|v| v.as_str())
            .map(|shift_key| {
                let mut sd = d.clone();
                sd.key = shift_key.to_owned();
                sd.text = shift_key.to_owned();
                if let Some(c) = num_of("shiftKeyCode") {
                    sd.key_code = c;
                }
                sd
            });
        let mut with_shift = d.clone();
        with_shift.shifted = shifted.clone().map(Box::new);
        out.insert(code.clone(), with_shift);
        if let Some((_, names)) = aliases.iter().find(|(c, _)| c == code) {
            for alias in *names {
                out.insert((*alias).to_owned(), d.clone());
            }
        }
        if d.location != 0 {
            continue;
        }
        if d.key.chars().count() == 1 {
            out.insert(d.key.clone(), d.clone());
        }
        if let Some(sd) = shifted {
            out.insert(sd.key.clone(), sd);
        }
    }
    out
}

/// Looks up a key (code, key name or character) in the US layout.
pub fn key_description(name: &str) -> Option<KeyDescription> {
    layout().get(name).cloned()
}

/// Splits `"Control+Shift+A"` into keys (a lone or trailing `+` is a key).
pub fn split_shortcut(shortcut: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut building = String::new();
    for c in shortcut.chars() {
        if c == '+' && !building.is_empty() {
            keys.push(std::mem::take(&mut building));
        } else {
            building.push(c);
        }
    }
    keys.push(building);
    keys
}

/// `ControlOrMeta` → `Meta` on macOS, `Control` elsewhere.
pub fn resolve_smart_modifier(key: &str, is_mac: bool) -> String {
    if key == "ControlOrMeta" {
        if is_mac { "Meta" } else { "Control" }.to_owned()
    } else {
        key.to_owned()
    }
}

/// Keyboard state: pressed keys and modifiers.
#[derive(Debug, Clone, Default)]
pub struct Keyboard {
    pressed_modifiers: BTreeSet<String>,
    pressed_keys: BTreeSet<String>,
    is_mac: bool,
}

impl Keyboard {
    pub fn new(is_mac: bool) -> Self {
        Self {
            is_mac,
            ..Default::default()
        }
    }

    pub fn modifiers(&self) -> u32 {
        let mut mask = 0;
        for m in &self.pressed_modifiers {
            mask |= match m.as_str() {
                "Alt" => ALT,
                "Control" => CONTROL,
                "Meta" => META,
                "Shift" => SHIFT,
                _ => 0,
            };
        }
        mask
    }

    fn describe(&self, key: &str) -> Result<KeyDescription, String> {
        let name = resolve_smart_modifier(key, self.is_mac);
        let mut d = key_description(&name).ok_or_else(|| format!("Unknown key: \"{name}\""))?;
        if self.pressed_modifiers.contains("Shift")
            && let Some(shifted) = d.shifted.take()
        {
            d = *shifted;
        }
        d.shifted = None;
        let only_shift =
            self.pressed_modifiers.len() == 1 && self.pressed_modifiers.contains("Shift");
        if !self.pressed_modifiers.is_empty() && !only_shift {
            d.text.clear();
        }
        Ok(d)
    }

    /// `keyboard.down(key)`.
    pub fn down(&mut self, key: &str) -> Result<KeyEvent, String> {
        let d = self.describe(key)?;
        let auto_repeat = !self.pressed_keys.insert(d.code.clone());
        if MODIFIERS.contains(&d.key.as_str()) {
            self.pressed_modifiers.insert(d.key.clone());
        }
        Ok(KeyEvent {
            kind: if d.text.is_empty() {
                KeyEventType::RawKeyDown
            } else {
                KeyEventType::KeyDown
            },
            modifiers: self.modifiers(),
            windows_virtual_key_code: d.key_code_without_location,
            is_keypad: d.location == 3,
            code: d.code,
            key: d.key,
            text: d.text,
            auto_repeat,
            location: d.location,
        })
    }

    /// `keyboard.up(key)`.
    pub fn up(&mut self, key: &str) -> Result<KeyEvent, String> {
        let d = self.describe(key)?;
        if MODIFIERS.contains(&d.key.as_str()) {
            self.pressed_modifiers.remove(&d.key);
        }
        self.pressed_keys.remove(&d.code);
        Ok(KeyEvent {
            kind: KeyEventType::KeyUp,
            modifiers: self.modifiers(),
            windows_virtual_key_code: d.key_code_without_location,
            is_keypad: d.location == 3,
            code: d.code,
            key: d.key,
            text: String::new(),
            auto_repeat: false,
            location: d.location,
        })
    }
}

/// One step of `keyboard.type(text)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeStep {
    /// Press this key (it is on the layout).
    Press(String),
    /// Insert this text (not on the layout, e.g. `ü`).
    Insert(String),
}

/// Playwright's `keyboard.type`: layout characters are pressed, others inserted.
pub fn type_steps(text: &str) -> Vec<TypeStep> {
    text.chars()
        .map(|c| {
            let s = c.to_string();
            if layout().contains_key(&s) {
                TypeStep::Press(s)
            } else {
                TypeStep::Insert(s)
            }
        })
        .collect()
}

/// Mouse buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl MouseButton {
    pub fn name(self) -> &'static str {
        match self {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
            MouseButton::Back => "back",
            MouseButton::Forward => "forward",
        }
    }
}

/// Mouse event kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventType {
    Moved,
    Pressed,
    Released,
    Wheel,
}

/// One raw mouse event.
#[derive(Debug, Clone, PartialEq)]
pub struct MouseEvent {
    pub kind: MouseEventType,
    pub x: f64,
    pub y: f64,
    /// `none` for moves without a pressed button.
    pub button: &'static str,
    pub buttons: u32,
    pub click_count: u32,
    pub delta_x: f64,
    pub delta_y: f64,
}

/// Mouse state.
#[derive(Debug, Clone, Default)]
pub struct Mouse {
    x: f64,
    y: f64,
    last_button: Option<MouseButton>,
    buttons: BTreeSet<MouseButton>,
}

impl Mouse {
    pub fn position(&self) -> (f64, f64) {
        (self.x, self.y)
    }

    fn event(&self, kind: MouseEventType, x: f64, y: f64, click_count: u32) -> MouseEvent {
        MouseEvent {
            kind,
            x,
            y,
            button: self.last_button.map(MouseButton::name).unwrap_or("none"),
            buttons: buttons_mask(&self.buttons),
            click_count,
            delta_x: 0.0,
            delta_y: 0.0,
        }
    }

    /// `mouse.move(x, y, { steps })`.
    pub fn move_to(&mut self, x: f64, y: f64, steps: u32) -> Vec<MouseEvent> {
        let steps = steps.max(1);
        let (from_x, from_y) = (self.x, self.y);
        self.x = x;
        self.y = y;
        (1..=steps)
            .map(|i| {
                let f = i as f64 / steps as f64;
                self.event(
                    MouseEventType::Moved,
                    from_x + (x - from_x) * f,
                    from_y + (y - from_y) * f,
                    0,
                )
            })
            .collect()
    }

    /// `mouse.down({ button, clickCount })`.
    pub fn down(&mut self, button: MouseButton, click_count: u32) -> MouseEvent {
        self.last_button = Some(button);
        self.buttons.insert(button);
        self.event(MouseEventType::Pressed, self.x, self.y, click_count)
    }

    /// `mouse.up({ button, clickCount })`.
    pub fn up(&mut self, button: MouseButton, click_count: u32) -> MouseEvent {
        self.buttons.remove(&button);
        let mut e = self.event(MouseEventType::Released, self.x, self.y, click_count);
        e.button = button.name();
        self.last_button = None;
        e
    }

    /// `mouse.wheel(deltaX, deltaY)`.
    pub fn wheel(&self, delta_x: f64, delta_y: f64) -> MouseEvent {
        let mut e = self.event(MouseEventType::Wheel, self.x, self.y, 0);
        e.delta_x = delta_x;
        e.delta_y = delta_y;
        e
    }
}

/// Playwright's buttons mask.
pub fn buttons_mask(buttons: &BTreeSet<MouseButton>) -> u32 {
    buttons
        .iter()
        .map(|b| match b {
            MouseButton::Left => 1,
            MouseButton::Right => 2,
            MouseButton::Middle => 4,
            MouseButton::Back => 8,
            MouseButton::Forward => 16,
        })
        .sum()
}

type Layout = HashMap<String, KeyDescription>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_key_produces_text() {
        let mut k = Keyboard::new(false);
        let e = k.down("a").unwrap();
        assert_eq!(
            (
                e.kind,
                e.key.as_str(),
                e.code.as_str(),
                e.text.as_str(),
                e.windows_virtual_key_code
            ),
            (KeyEventType::KeyDown, "a", "KeyA", "a", 65)
        );
        let e = k.up("a").unwrap();
        assert_eq!((e.kind, e.text.as_str()), (KeyEventType::KeyUp, ""));
    }

    #[test]
    fn shift_changes_key_and_text() {
        let mut k = Keyboard::new(false);
        let s = k.down("Shift").unwrap();
        assert_eq!(
            (s.kind, s.modifiers, s.code.as_str()),
            (KeyEventType::RawKeyDown, SHIFT, "ShiftLeft")
        );
        let e = k.down("KeyA").unwrap();
        assert_eq!(
            (e.key.as_str(), e.text.as_str(), e.modifiers),
            ("A", "A", SHIFT)
        );
        // Like Playwright, the character "a" has no shifted variant.
        assert_eq!(k.down("a").unwrap().key, "a");
        let e = k.down("Digit1").unwrap();
        assert_eq!(e.key, "!");
    }

    #[test]
    fn control_suppresses_text() {
        let mut k = Keyboard::new(false);
        k.down("Control").unwrap();
        let e = k.down("a").unwrap();
        assert_eq!(
            (e.kind, e.text.as_str(), e.modifiers),
            (KeyEventType::RawKeyDown, "", CONTROL)
        );
        k.up("Control").unwrap();
        assert_eq!(k.modifiers(), 0);
    }

    #[test]
    fn enter_aliases_keypad_and_autorepeat() {
        let mut k = Keyboard::new(false);
        let e = k.down("\n").unwrap();
        assert_eq!(
            (
                e.key.as_str(),
                e.code.as_str(),
                e.text.as_str(),
                e.windows_virtual_key_code
            ),
            ("Enter", "Enter", "\r", 13)
        );
        assert!(!e.auto_repeat);
        assert!(k.down("Enter").unwrap().auto_repeat);
        let n = k.down("Numpad1").unwrap();
        assert_eq!((n.location, n.is_keypad, n.key.as_str()), (3, true, "End"));
        assert!(key_description("Foo").is_none());
        assert_eq!(k.down("Foo").unwrap_err(), "Unknown key: \"Foo\"");
    }

    #[test]
    fn shortcuts_and_smart_modifiers() {
        assert_eq!(
            split_shortcut("Control+Shift+A"),
            vec!["Control", "Shift", "A"]
        );
        assert_eq!(split_shortcut("Control++"), vec!["Control", "+"]);
        assert_eq!(split_shortcut("+"), vec!["+"]);
        assert_eq!(resolve_smart_modifier("ControlOrMeta", true), "Meta");
        assert_eq!(resolve_smart_modifier("ControlOrMeta", false), "Control");
        assert_eq!(resolve_smart_modifier("a", true), "a");
        let mut mac = Keyboard::new(true);
        assert_eq!(mac.down("ControlOrMeta").unwrap().modifiers, META);
    }

    #[test]
    fn typing_presses_layout_keys_and_inserts_the_rest() {
        assert_eq!(
            type_steps("Hü!"),
            vec![
                TypeStep::Press("H".into()),
                TypeStep::Insert("ü".into()),
                TypeStep::Press("!".into())
            ]
        );
    }

    #[test]
    fn mouse_moves_in_steps_and_tracks_buttons() {
        let mut m = Mouse::default();
        let moves = m.move_to(10.0, 20.0, 2);
        assert_eq!(moves.len(), 2);
        assert_eq!(
            (moves[0].x, moves[0].y, moves[0].button),
            (5.0, 10.0, "none")
        );
        assert_eq!((moves[1].x, moves[1].y), (10.0, 20.0));
        let d = m.down(MouseButton::Left, 1);
        assert_eq!(
            (d.kind, d.button, d.buttons, d.click_count, d.x),
            (MouseEventType::Pressed, "left", 1, 1, 10.0)
        );
        let drag = m.move_to(30.0, 20.0, 1);
        assert_eq!((drag[0].button, drag[0].buttons), ("left", 1));
        let u = m.up(MouseButton::Left, 1);
        assert_eq!((u.kind, u.buttons), (MouseEventType::Released, 0));
        assert_eq!(m.wheel(0.0, 100.0).delta_y, 100.0);
        let mut set = BTreeSet::new();
        set.insert(MouseButton::Right);
        set.insert(MouseButton::Middle);
        assert_eq!(buttons_mask(&set), 6);
    }
}
