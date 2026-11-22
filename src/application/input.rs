//! Page keyboard, mouse and touchscreen (Playwright's `page.keyboard` etc.).

use serde_json::{Value, json};

use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::input::{
    KeyEvent, KeyEventType, MouseButton, MouseEvent, MouseEventType, TypeStep, split_shortcut,
    type_steps,
};

/// `page.keyboard`.
#[derive(Debug, Clone)]
pub struct Keyboard {
    page: Page,
}

/// `page.mouse`.
#[derive(Debug, Clone)]
pub struct Mouse {
    page: Page,
}

/// `page.touchscreen`.
#[derive(Debug, Clone)]
pub struct Touchscreen {
    page: Page,
}

impl Page {
    pub fn keyboard(&self) -> Keyboard {
        Keyboard { page: self.clone() }
    }

    pub fn mouse(&self) -> Mouse {
        Mouse { page: self.clone() }
    }

    pub fn touchscreen(&self) -> Touchscreen {
        Touchscreen { page: self.clone() }
    }
}

fn key_params(e: &KeyEvent) -> Value {
    match e.kind {
        KeyEventType::KeyUp => json!({
            "type": "keyUp",
            "modifiers": e.modifiers,
            "key": e.key,
            "windowsVirtualKeyCode": e.windows_virtual_key_code,
            "code": e.code,
            "location": e.location,
        }),
        kind => json!({
            "type": if kind == KeyEventType::KeyDown { "keyDown" } else { "rawKeyDown" },
            "modifiers": e.modifiers,
            "windowsVirtualKeyCode": e.windows_virtual_key_code,
            "code": e.code,
            "commands": [],
            "key": e.key,
            "text": e.text,
            "unmodifiedText": e.text,
            "autoRepeat": e.auto_repeat,
            "location": e.location,
            "isKeypad": e.is_keypad,
        }),
    }
}

impl Keyboard {
    /// `keyboard.down(key)`.
    pub async fn down(&self, key: &str) -> Result<()> {
        let event = self
            .page
            .inner
            .keyboard
            .lock()
            .await
            .down(key)
            .map_err(Error::InvalidArgument)?;
        self.page
            .send("Input.dispatchKeyEvent", key_params(&event))
            .await?;
        Ok(())
    }

    /// `keyboard.up(key)`.
    pub async fn up(&self, key: &str) -> Result<()> {
        let event = self
            .page
            .inner
            .keyboard
            .lock()
            .await
            .up(key)
            .map_err(Error::InvalidArgument)?;
        self.page
            .send("Input.dispatchKeyEvent", key_params(&event))
            .await?;
        Ok(())
    }

    /// `keyboard.press("Control+Shift+A")`.
    pub async fn press(&self, shortcut: &str) -> Result<()> {
        let keys = split_shortcut(shortcut);
        let (last, modifiers) = keys.split_last().expect("at least one key");
        for m in modifiers {
            self.down(m).await?;
        }
        self.down(last).await?;
        self.up(last).await?;
        for m in modifiers.iter().rev() {
            self.up(m).await?;
        }
        Ok(())
    }

    /// `keyboard.type(text)` / `pressSequentially`: presses layout keys,
    /// inserts other characters.
    pub async fn type_text(&self, text: &str) -> Result<()> {
        for step in type_steps(text) {
            match step {
                TypeStep::Press(key) => self.press_single(&key).await?,
                TypeStep::Insert(text) => self.insert_text(&text).await?,
            }
        }
        Ok(())
    }

    async fn press_single(&self, key: &str) -> Result<()> {
        self.down(key).await?;
        self.up(key).await
    }

    /// `keyboard.insertText(text)`: input event only, no key events.
    pub async fn insert_text(&self, text: &str) -> Result<()> {
        self.page
            .send("Input.insertText", json!({"text": text}))
            .await?;
        Ok(())
    }
}

impl Mouse {
    async fn dispatch(&self, e: &MouseEvent) -> Result<()> {
        let modifiers = self.page.inner.keyboard.lock().await.modifiers();
        let params = match e.kind {
            MouseEventType::Moved => json!({
                "type": "mouseMoved", "button": e.button, "buttons": e.buttons, "x": e.x, "y": e.y,
                "modifiers": modifiers, "force": if e.buttons > 0 { 0.5 } else { 0.0 },
            }),
            MouseEventType::Pressed => json!({
                "type": "mousePressed", "button": e.button, "buttons": e.buttons, "x": e.x, "y": e.y,
                "modifiers": modifiers, "clickCount": e.click_count, "force": if e.buttons > 0 { 0.5 } else { 0.0 },
            }),
            MouseEventType::Released => json!({
                "type": "mouseReleased", "button": e.button, "buttons": e.buttons, "x": e.x, "y": e.y,
                "modifiers": modifiers, "clickCount": e.click_count,
            }),
            MouseEventType::Wheel => json!({
                "type": "mouseWheel", "x": e.x, "y": e.y, "modifiers": modifiers,
                "deltaX": e.delta_x, "deltaY": e.delta_y,
            }),
        };
        self.page.send("Input.dispatchMouseEvent", params).await?;
        Ok(())
    }

    /// `mouse.move(x, y, { steps })`.
    pub async fn move_to(&self, x: f64, y: f64, steps: u32) -> Result<()> {
        let events = self.page.inner.mouse.lock().await.move_to(x, y, steps);
        for e in &events {
            self.dispatch(e).await?;
        }
        Ok(())
    }

    /// `mouse.down({ button })`.
    pub async fn down(&self, button: MouseButton) -> Result<()> {
        self.down_n(button, 1).await
    }

    async fn down_n(&self, button: MouseButton, click_count: u32) -> Result<()> {
        let e = self.page.inner.mouse.lock().await.down(button, click_count);
        self.dispatch(&e).await
    }

    /// `mouse.up({ button })`.
    pub async fn up(&self, button: MouseButton) -> Result<()> {
        self.up_n(button, 1).await
    }

    async fn up_n(&self, button: MouseButton, click_count: u32) -> Result<()> {
        let e = self.page.inner.mouse.lock().await.up(button, click_count);
        self.dispatch(&e).await
    }

    /// `mouse.click(x, y, { button, clickCount })`.
    pub async fn click_with(
        &self,
        x: f64,
        y: f64,
        button: MouseButton,
        click_count: u32,
    ) -> Result<()> {
        self.move_to(x, y, 1).await?;
        for n in 1..=click_count.max(1) {
            self.down_n(button, n).await?;
            self.up_n(button, n).await?;
        }
        Ok(())
    }

    /// `mouse.click(x, y)`.
    pub async fn click(&self, x: f64, y: f64) -> Result<()> {
        self.click_with(x, y, MouseButton::Left, 1).await
    }

    /// `mouse.dblclick(x, y)`.
    pub async fn dblclick(&self, x: f64, y: f64) -> Result<()> {
        self.click_with(x, y, MouseButton::Left, 2).await
    }

    /// `mouse.wheel(deltaX, deltaY)`.
    pub async fn wheel(&self, delta_x: f64, delta_y: f64) -> Result<()> {
        let e = self.page.inner.mouse.lock().await.wheel(delta_x, delta_y);
        self.dispatch(&e).await
    }
}

impl Touchscreen {
    /// `touchscreen.tap(x, y)`.
    pub async fn tap(&self, x: f64, y: f64) -> Result<()> {
        let modifiers = self.page.inner.keyboard.lock().await.modifiers();
        let start = self.page.send(
            "Input.dispatchTouchEvent",
            json!({"type": "touchStart", "modifiers": modifiers, "touchPoints": [{"x": x, "y": y}]}),
        );
        let end = self.page.send(
            "Input.dispatchTouchEvent",
            json!({"type": "touchEnd", "modifiers": modifiers, "touchPoints": []}),
        );
        let (a, b) = futures::join!(start, end);
        a?;
        b?;
        Ok(())
    }
}
