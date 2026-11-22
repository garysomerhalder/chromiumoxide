//! Page keyboard, mouse and touchscreen (Playwright's `page.keyboard` etc.).

use futures::StreamExt;
use serde_json::{Value, json};

use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::frame::World;
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

/// Watches for a drag starting on the next mouse move (Playwright's
/// `setupDragListeners`, run in every frame's utility world).
const SETUP_DRAG: &str = "function() {
  let didStartDrag = Promise.resolve(false);
  let dragEvent = null;
  const dragListener = (event) => dragEvent = event;
  const mouseListener = () => {
    didStartDrag = new Promise((callback) => {
      window.addEventListener('dragstart', dragListener, { once: true, capture: true });
      setTimeout(() => callback(dragEvent ? !dragEvent.defaultPrevented : false), 0);
    });
  };
  window.addEventListener('mousemove', mouseListener, { once: true, capture: true });
  window.__cleanupDrag = async () => {
    const val = await didStartDrag;
    window.removeEventListener('mousemove', mouseListener, { capture: true });
    window.removeEventListener('dragstart', dragListener, { capture: true });
    delete window.__cleanupDrag;
    return val;
  };
}";

const CLEANUP_DRAG: &str =
    "function() { return window.__cleanupDrag ? window.__cleanupDrag() : false; }";

impl Mouse {
    async fn modifiers(&self) -> u32 {
        self.page.inner.keyboard.lock().await.modifiers()
    }

    fn drag_state(&self) -> Option<Value> {
        self.page.inner.drag.lock().unwrap().clone()
    }

    async fn drag_event(&self, kind: &str, x: f64, y: f64, data: Value) -> Result<()> {
        let modifiers = self.modifiers().await;
        self.page
            .send(
                "Input.dispatchDragEvent",
                json!({"type": kind, "x": x, "y": y, "data": data, "modifiers": modifiers}),
            )
            .await?;
        Ok(())
    }

    /// A move that may start or continue an HTML5 drag (Playwright's
    /// `DragManager.interceptDragCausedByMove`).
    async fn dispatch_move(&self, e: &MouseEvent) -> Result<()> {
        if let Some(data) = self.drag_state() {
            return self.drag_event("dragOver", e.x, e.y, data).await;
        }
        if e.button != "left" {
            return self.dispatch(e).await;
        }
        let timeout = self.page.default_timeout();
        let frames = self.page.frames();
        for f in &frames {
            let _ = self
                .page
                .call_function_in(
                    Some(f.id()),
                    World::Utility,
                    SETUP_DRAG,
                    &[],
                    timeout,
                    "watching drags",
                )
                .await;
        }
        let mut cdp = self.page.inner.shared.cdp.subscribe();
        self.page
            .send("Input.setInterceptDrags", json!({"enabled": true}))
            .await?;
        let moved = self.dispatch(e).await;
        let mut expecting = false;
        for f in &frames {
            let started = self
                .page
                .call_function_in(
                    Some(f.id()),
                    World::Utility,
                    CLEANUP_DRAG,
                    &[],
                    timeout,
                    "watching drags",
                )
                .await;
            expecting |= matches!(started, Ok(Value::Bool(true)));
        }
        let mut data = None;
        if expecting && moved.is_ok() {
            let session = self.page.inner.session_id.clone();
            let wait = async {
                while let Some(ev) = cdp.next().await {
                    if ev.session_id.as_deref() == Some(session.as_str())
                        && ev.method == "Input.dragIntercepted"
                    {
                        return Some(ev.params["data"].clone());
                    }
                }
                None
            };
            let clock = self.page.inner.shared.services.clock.clone();
            data = tokio::select! {
                d = wait => d,
                _ = clock.sleep(timeout) => None,
            };
        }
        let _ = self
            .page
            .send("Input.setInterceptDrags", json!({"enabled": false}))
            .await;
        moved?;
        if let Some(data) = data {
            *self.page.inner.drag.lock().unwrap() = Some(data.clone());
            self.drag_event("dragEnter", e.x, e.y, data).await?;
        }
        Ok(())
    }

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
            self.dispatch_move(e).await?;
        }
        Ok(())
    }

    /// The move of a click: never starts a drag.
    async fn move_for_click(&self, x: f64, y: f64) -> Result<()> {
        let events = self.page.inner.mouse.lock().await.move_to(x, y, 1);
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
        if self.drag_state().is_some() {
            return Ok(());
        }
        self.dispatch(&e).await
    }

    /// `mouse.up({ button })`.
    pub async fn up(&self, button: MouseButton) -> Result<()> {
        self.up_n(button, 1).await
    }

    async fn up_n(&self, button: MouseButton, click_count: u32) -> Result<()> {
        let e = self.page.inner.mouse.lock().await.up(button, click_count);
        let drag = self.page.inner.drag.lock().unwrap().take();
        if let Some(data) = drag {
            return self.drag_event("drop", e.x, e.y, data).await;
        }
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
        self.move_for_click(x, y).await?;
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
