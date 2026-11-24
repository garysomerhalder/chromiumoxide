//! Raw input on the Firefox engine (`page.keyboard()`, `page.mouse()`):
//! BiDi `input.performActions`, driven by the shared keyboard model
//! (`domain::input`) so keys come out as in Playwright (Shift+a is "A").

use std::sync::Arc;

use serde_json::{Value, json};

use crate::application::firefox::Page;
use crate::domain::error::{Error, Result};
use crate::domain::firefox::webdriver_key;
use crate::domain::input::{MouseButton, TypeStep, split_shortcut, type_steps};

/// The pointer source id shared by locator actions and `page.mouse()`, so
/// BiDi keeps one pointer state (position, pressed buttons) per page.
pub(crate) const MOUSE: &str = "__chromiumoxide_mouse";
/// The key source id.
pub(crate) const KEYBOARD: &str = "__chromiumoxide_keyboard";

/// Per-page input state.
#[derive(Default)]
pub(crate) struct InputState {
    pub(crate) keyboard: tokio::sync::Mutex<crate::domain::input::Keyboard>,
    pub(crate) mouse: std::sync::Mutex<(f64, f64)>,
}

/// The WebDriver value of a key event's key.
fn key_value(key: &str) -> Result<String> {
    if key.chars().count() == 1 {
        return Ok(key.to_owned());
    }
    webdriver_key(key).ok_or_else(|| Error::InvalidArgument(format!("Unknown key: \"{key}\"")))
}

/// `page.keyboard()` on Firefox.
#[derive(Clone)]
pub struct Keyboard {
    page: Page,
    state: Arc<InputState>,
}

impl Keyboard {
    pub(crate) fn new(page: Page, state: Arc<InputState>) -> Self {
        Keyboard { page, state }
    }

    async fn send(&self, actions: Vec<Value>) -> Result<()> {
        self.page
            .perform_actions(json!([{"type": "key", "id": KEYBOARD, "actions": actions}]))
            .await
    }

    /// `keyboard.down(key)`.
    pub async fn down(&self, key: &str) -> Result<()> {
        let event = self
            .state
            .keyboard
            .lock()
            .await
            .down(key)
            .map_err(Error::InvalidArgument)?;
        self.send(vec![
            json!({"type": "keyDown", "value": key_value(&event.key)?}),
        ])
        .await
    }

    /// `keyboard.up(key)`.
    pub async fn up(&self, key: &str) -> Result<()> {
        let event = self
            .state
            .keyboard
            .lock()
            .await
            .up(key)
            .map_err(Error::InvalidArgument)?;
        self.send(vec![
            json!({"type": "keyUp", "value": key_value(&event.key)?}),
        ])
        .await
    }

    /// `keyboard.press(key)`: `"Enter"`, `"Control+a"`, ...
    pub async fn press(&self, shortcut: &str) -> Result<()> {
        let keys = split_shortcut(shortcut);
        for key in &keys {
            self.down(key).await?;
        }
        for key in keys.iter().rev() {
            self.up(key).await?;
        }
        Ok(())
    }

    /// `keyboard.type(text)`: keys on the layout are pressed, other
    /// characters inserted.
    pub async fn type_text(&self, text: &str) -> Result<()> {
        for step in type_steps(text) {
            match step {
                TypeStep::Press(key) => self.press(&key).await?,
                TypeStep::Insert(text) => self.insert_text(&text).await?,
            }
        }
        Ok(())
    }

    /// `keyboard.insertText(text)`: text without key events.
    pub async fn insert_text(&self, text: &str) -> Result<()> {
        let context = self.page.main_context();
        self.page.insert_text_in(&context, text).await
    }
}

fn button_number(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
    }
}

/// `page.mouse()` on Firefox (viewport CSS pixels).
#[derive(Clone)]
pub struct Mouse {
    page: Page,
    state: Arc<InputState>,
}

impl Mouse {
    pub(crate) fn new(page: Page, state: Arc<InputState>) -> Self {
        Mouse { page, state }
    }

    async fn pointer(&self, actions: Vec<Value>) -> Result<()> {
        self.page
            .perform_actions(json!([{"type": "pointer", "id": MOUSE,
                "parameters": {"pointerType": "mouse"}, "actions": actions}]))
            .await
    }

    /// `mouse.move(x, y, { steps })`.
    pub async fn move_to(&self, x: f64, y: f64, steps: u32) -> Result<()> {
        let from = *self.state.mouse.lock().unwrap();
        let steps = steps.max(1);
        let actions = (1..=steps)
            .map(|i| {
                let t = f64::from(i) / f64::from(steps);
                json!({"type": "pointerMove",
                       "x": (from.0 + (x - from.0) * t).round() as i64,
                       "y": (from.1 + (y - from.1) * t).round() as i64})
            })
            .collect();
        self.pointer(actions).await?;
        *self.state.mouse.lock().unwrap() = (x, y);
        Ok(())
    }

    /// `mouse.down({ button })`.
    pub async fn down(&self, button: MouseButton) -> Result<()> {
        self.pointer(vec![
            json!({"type": "pointerDown", "button": button_number(button)}),
        ])
        .await
    }

    /// `mouse.up({ button })`.
    pub async fn up(&self, button: MouseButton) -> Result<()> {
        self.pointer(vec![
            json!({"type": "pointerUp", "button": button_number(button)}),
        ])
        .await
    }

    /// `mouse.click(x, y)`.
    pub async fn click(&self, x: f64, y: f64) -> Result<()> {
        self.move_to(x, y, 1).await?;
        self.pointer(vec![
            json!({"type": "pointerDown", "button": 0}),
            json!({"type": "pointerUp", "button": 0}),
        ])
        .await
    }

    /// `mouse.dblclick(x, y)`.
    pub async fn dblclick(&self, x: f64, y: f64) -> Result<()> {
        self.move_to(x, y, 1).await?;
        self.pointer(vec![
            json!({"type": "pointerDown", "button": 0}),
            json!({"type": "pointerUp", "button": 0}),
            json!({"type": "pointerDown", "button": 0}),
            json!({"type": "pointerUp", "button": 0}),
        ])
        .await
    }

    /// Presses at `(x, y)`, moves to `(tx, ty)` and releases, as one action
    /// chain so Firefox runs it as a drag session.
    pub(crate) async fn drag(&self, x: f64, y: f64, tx: f64, ty: f64) -> Result<()> {
        let (x, y, tx, ty) = (
            x.round() as i64,
            y.round() as i64,
            tx.round() as i64,
            ty.round() as i64,
        );
        let mut actions = vec![
            json!({"type": "pointerMove", "x": x, "y": y}),
            json!({"type": "pointerDown", "button": 0}),
            json!({"type": "pointerMove", "x": x + 5, "y": y + 5, "duration": 50}),
        ];
        for i in 1..=5 {
            actions.push(json!({"type": "pointerMove",
                "x": x + (tx - x) * i / 5, "y": y + (ty - y) * i / 5, "duration": 50}));
        }
        actions.push(json!({"type": "pointerUp", "button": 0}));
        self.pointer(actions).await?;
        *self.state.mouse.lock().unwrap() = (tx as f64, ty as f64);
        Ok(())
    }

    /// `mouse.wheel(deltaX, deltaY)` at the mouse position.
    pub async fn wheel(&self, delta_x: f64, delta_y: f64) -> Result<()> {
        let (x, y) = *self.state.mouse.lock().unwrap();
        self.page
            .perform_actions(
                json!([{"type": "wheel", "id": "__chromiumoxide_wheel", "actions": [
                    {"type": "scroll", "x": x.round() as i64, "y": y.round() as i64,
                     "deltaX": delta_x.round() as i64, "deltaY": delta_y.round() as i64}
                ]}]),
            )
            .await
    }
}
