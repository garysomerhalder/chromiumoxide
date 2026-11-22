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

/// Looks up a key (code, key name or character) in the US layout.
pub fn key_description(_name: &str) -> Option<KeyDescription> {
    unimplemented!()
}

/// Splits `"Control+Shift+A"` into keys (a lone or trailing `+` is a key).
pub fn split_shortcut(_shortcut: &str) -> Vec<String> {
    unimplemented!()
}

/// `ControlOrMeta` → `Meta` on macOS, `Control` elsewhere.
pub fn resolve_smart_modifier(_key: &str, _is_mac: bool) -> String {
    unimplemented!()
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
        unimplemented!()
    }

    /// `keyboard.down(key)`.
    pub fn down(&mut self, _key: &str) -> Result<KeyEvent, String> {
        unimplemented!()
    }

    /// `keyboard.up(key)`.
    pub fn up(&mut self, _key: &str) -> Result<KeyEvent, String> {
        unimplemented!()
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
pub fn type_steps(_text: &str) -> Vec<TypeStep> {
    unimplemented!()
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

    /// `mouse.move(x, y, { steps })`.
    pub fn move_to(&mut self, _x: f64, _y: f64, _steps: u32) -> Vec<MouseEvent> {
        unimplemented!()
    }

    /// `mouse.down({ button, clickCount })`.
    pub fn down(&mut self, _button: MouseButton, _click_count: u32) -> MouseEvent {
        unimplemented!()
    }

    /// `mouse.up({ button, clickCount })`.
    pub fn up(&mut self, _button: MouseButton, _click_count: u32) -> MouseEvent {
        unimplemented!()
    }

    /// `mouse.wheel(deltaX, deltaY)`.
    pub fn wheel(&self, _delta_x: f64, _delta_y: f64) -> MouseEvent {
        unimplemented!()
    }
}

/// Playwright's buttons mask.
pub fn buttons_mask(_buttons: &BTreeSet<MouseButton>) -> u32 {
    unimplemented!()
}

#[allow(dead_code)]
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
        let e = k.down("a").unwrap();
        assert_eq!(
            (e.key.as_str(), e.text.as_str(), e.modifiers),
            ("A", "A", SHIFT)
        );
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
