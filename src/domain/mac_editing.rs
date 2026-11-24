//! Playwright's macOS editing commands (`macEditingCommands`): the Cocoa
//! selectors a key press runs in a text field on a Mac (Backspace is
//! `deleteBackward:`, Meta+A `selectAll:`). Browsers driven without a real
//! keyboard need them sent with the key: Chromium takes them as `commands`
//! (Playwright sends them on macOS only, without the `insert…` ones and the
//! trailing colon), WebKit as `macCommands`.

/// Playwright's table, by shortcut (`Shift`, `Control`, `Alt`, `Meta`, then
/// the key code, joined with `+`).
const COMMANDS: &[(&str, &[&str])] = &[
    ("Backspace", &["deleteBackward:"]),
    ("Enter", &["insertNewline:"]),
    ("NumpadEnter", &["insertNewline:"]),
    ("Escape", &["cancelOperation:"]),
    ("ArrowUp", &["moveUp:"]),
    ("ArrowDown", &["moveDown:"]),
    ("ArrowLeft", &["moveLeft:"]),
    ("ArrowRight", &["moveRight:"]),
    ("F5", &["complete:"]),
    ("Delete", &["deleteForward:"]),
    ("Home", &["scrollToBeginningOfDocument:"]),
    ("End", &["scrollToEndOfDocument:"]),
    ("PageUp", &["scrollPageUp:"]),
    ("PageDown", &["scrollPageDown:"]),
    ("Shift+Backspace", &["deleteBackward:"]),
    ("Shift+Enter", &["insertNewline:"]),
    ("Shift+NumpadEnter", &["insertNewline:"]),
    ("Shift+Escape", &["cancelOperation:"]),
    ("Shift+ArrowUp", &["moveUpAndModifySelection:"]),
    ("Shift+ArrowDown", &["moveDownAndModifySelection:"]),
    ("Shift+ArrowLeft", &["moveLeftAndModifySelection:"]),
    ("Shift+ArrowRight", &["moveRightAndModifySelection:"]),
    ("Shift+F5", &["complete:"]),
    ("Shift+Delete", &["deleteForward:"]),
    (
        "Shift+Home",
        &["moveToBeginningOfDocumentAndModifySelection:"],
    ),
    ("Shift+End", &["moveToEndOfDocumentAndModifySelection:"]),
    ("Shift+PageUp", &["pageUpAndModifySelection:"]),
    ("Shift+PageDown", &["pageDownAndModifySelection:"]),
    ("Shift+Numpad5", &["delete:"]),
    ("Control+Tab", &["selectNextKeyView:"]),
    ("Control+Enter", &["insertLineBreak:"]),
    ("Control+NumpadEnter", &["insertLineBreak:"]),
    ("Control+Quote", &["insertSingleQuoteIgnoringSubstitution:"]),
    ("Control+KeyA", &["moveToBeginningOfParagraph:"]),
    ("Control+KeyB", &["moveBackward:"]),
    ("Control+KeyD", &["deleteForward:"]),
    ("Control+KeyE", &["moveToEndOfParagraph:"]),
    ("Control+KeyF", &["moveForward:"]),
    ("Control+KeyH", &["deleteBackward:"]),
    ("Control+KeyK", &["deleteToEndOfParagraph:"]),
    ("Control+KeyL", &["centerSelectionInVisibleArea:"]),
    ("Control+KeyN", &["moveDown:"]),
    (
        "Control+KeyO",
        &["insertNewlineIgnoringFieldEditor:", "moveBackward:"],
    ),
    ("Control+KeyP", &["moveUp:"]),
    ("Control+KeyT", &["transpose:"]),
    ("Control+KeyV", &["pageDown:"]),
    ("Control+KeyY", &["yank:"]),
    (
        "Control+Backspace",
        &["deleteBackwardByDecomposingPreviousCharacter:"],
    ),
    ("Control+ArrowUp", &["scrollPageUp:"]),
    ("Control+ArrowDown", &["scrollPageDown:"]),
    ("Control+ArrowLeft", &["moveToLeftEndOfLine:"]),
    ("Control+ArrowRight", &["moveToRightEndOfLine:"]),
    ("Shift+Control+Enter", &["insertLineBreak:"]),
    ("Shift+Control+NumpadEnter", &["insertLineBreak:"]),
    ("Shift+Control+Tab", &["selectPreviousKeyView:"]),
    (
        "Shift+Control+Quote",
        &["insertDoubleQuoteIgnoringSubstitution:"],
    ),
    (
        "Shift+Control+KeyA",
        &["moveToBeginningOfParagraphAndModifySelection:"],
    ),
    ("Shift+Control+KeyB", &["moveBackwardAndModifySelection:"]),
    (
        "Shift+Control+KeyE",
        &["moveToEndOfParagraphAndModifySelection:"],
    ),
    ("Shift+Control+KeyF", &["moveForwardAndModifySelection:"]),
    ("Shift+Control+KeyN", &["moveDownAndModifySelection:"]),
    ("Shift+Control+KeyP", &["moveUpAndModifySelection:"]),
    ("Shift+Control+KeyV", &["pageDownAndModifySelection:"]),
    (
        "Shift+Control+Backspace",
        &["deleteBackwardByDecomposingPreviousCharacter:"],
    ),
    ("Shift+Control+ArrowUp", &["scrollPageUp:"]),
    ("Shift+Control+ArrowDown", &["scrollPageDown:"]),
    (
        "Shift+Control+ArrowLeft",
        &["moveToLeftEndOfLineAndModifySelection:"],
    ),
    (
        "Shift+Control+ArrowRight",
        &["moveToRightEndOfLineAndModifySelection:"],
    ),
    ("Alt+Backspace", &["deleteWordBackward:"]),
    ("Alt+Enter", &["insertNewlineIgnoringFieldEditor:"]),
    ("Alt+NumpadEnter", &["insertNewlineIgnoringFieldEditor:"]),
    ("Alt+Escape", &["complete:"]),
    (
        "Alt+ArrowUp",
        &["moveBackward:", "moveToBeginningOfParagraph:"],
    ),
    ("Alt+ArrowDown", &["moveForward:", "moveToEndOfParagraph:"]),
    ("Alt+ArrowLeft", &["moveWordLeft:"]),
    ("Alt+ArrowRight", &["moveWordRight:"]),
    ("Alt+Delete", &["deleteWordForward:"]),
    ("Alt+PageUp", &["pageUp:"]),
    ("Alt+PageDown", &["pageDown:"]),
    ("Shift+Alt+Backspace", &["deleteWordBackward:"]),
    ("Shift+Alt+Enter", &["insertNewlineIgnoringFieldEditor:"]),
    (
        "Shift+Alt+NumpadEnter",
        &["insertNewlineIgnoringFieldEditor:"],
    ),
    ("Shift+Alt+Escape", &["complete:"]),
    (
        "Shift+Alt+ArrowUp",
        &["moveParagraphBackwardAndModifySelection:"],
    ),
    (
        "Shift+Alt+ArrowDown",
        &["moveParagraphForwardAndModifySelection:"],
    ),
    ("Shift+Alt+ArrowLeft", &["moveWordLeftAndModifySelection:"]),
    (
        "Shift+Alt+ArrowRight",
        &["moveWordRightAndModifySelection:"],
    ),
    ("Shift+Alt+Delete", &["deleteWordForward:"]),
    ("Shift+Alt+PageUp", &["pageUp:"]),
    ("Shift+Alt+PageDown", &["pageDown:"]),
    ("Control+Alt+KeyB", &["moveWordBackward:"]),
    ("Control+Alt+KeyF", &["moveWordForward:"]),
    ("Control+Alt+Backspace", &["deleteWordBackward:"]),
    (
        "Shift+Control+Alt+KeyB",
        &["moveWordBackwardAndModifySelection:"],
    ),
    (
        "Shift+Control+Alt+KeyF",
        &["moveWordForwardAndModifySelection:"],
    ),
    ("Shift+Control+Alt+Backspace", &["deleteWordBackward:"]),
    ("Meta+NumpadSubtract", &["cancel:"]),
    ("Meta+Backspace", &["deleteToBeginningOfLine:"]),
    ("Meta+ArrowUp", &["moveToBeginningOfDocument:"]),
    ("Meta+ArrowDown", &["moveToEndOfDocument:"]),
    ("Meta+ArrowLeft", &["moveToLeftEndOfLine:"]),
    ("Meta+ArrowRight", &["moveToRightEndOfLine:"]),
    ("Shift+Meta+NumpadSubtract", &["cancel:"]),
    ("Shift+Meta+Backspace", &["deleteToBeginningOfLine:"]),
    (
        "Shift+Meta+ArrowUp",
        &["moveToBeginningOfDocumentAndModifySelection:"],
    ),
    (
        "Shift+Meta+ArrowDown",
        &["moveToEndOfDocumentAndModifySelection:"],
    ),
    (
        "Shift+Meta+ArrowLeft",
        &["moveToLeftEndOfLineAndModifySelection:"],
    ),
    (
        "Shift+Meta+ArrowRight",
        &["moveToRightEndOfLineAndModifySelection:"],
    ),
    ("Meta+KeyA", &["selectAll:"]),
    ("Meta+KeyC", &["copy:"]),
    ("Meta+KeyX", &["cut:"]),
    ("Meta+KeyV", &["paste:"]),
    ("Meta+KeyZ", &["undo:"]),
    ("Shift+Meta+KeyZ", &["redo:"]),
];

/// CDP modifier bits.
const ALT: u32 = 1;
const CONTROL: u32 = 2;
const META: u32 = 4;
const SHIFT: u32 = 8;

/// Playwright's shortcut name for `code` with CDP `modifiers`.
pub fn shortcut(code: &str, modifiers: u32) -> String {
    let mut parts: Vec<&str> = [
        (SHIFT, "Shift"),
        (CONTROL, "Control"),
        (ALT, "Alt"),
        (META, "Meta"),
    ]
    .into_iter()
    .filter(|(bit, _)| modifiers & bit != 0)
    .map(|(_, name)| name)
    .collect();
    parts.push(code);
    parts.join("+")
}

/// The commands for a shortcut, as in Playwright's table.
pub fn commands(shortcut: &str) -> &'static [&'static str] {
    COMMANDS
        .iter()
        .find(|(s, _)| *s == shortcut)
        .map(|(_, c)| *c)
        .unwrap_or(&[])
}

/// Chromium's `Input.dispatchKeyEvent` `commands` on macOS.
pub fn chromium_commands(code: &str, modifiers: u32) -> Vec<String> {
    commands(&shortcut(code, modifiers))
        .iter()
        .filter(|c| !c.starts_with("insert"))
        .map(|c| c.trim_end_matches(':').to_owned())
        .collect()
}

/// WebKit's `Input.dispatchKeyEvent` `macCommands`.
pub fn webkit_commands(code: &str, modifiers: u32) -> Vec<String> {
    commands(&shortcut(code, modifiers))
        .iter()
        .map(|c| (*c).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_follow_playwrights_modifier_order() {
        assert_eq!(shortcut("KeyZ", SHIFT | META), "Shift+Meta+KeyZ");
        assert_eq!(
            shortcut("ArrowLeft", ALT | CONTROL),
            "Control+Alt+ArrowLeft"
        );
        assert_eq!(shortcut("Backspace", 0), "Backspace");
    }

    #[test]
    fn playwrights_table() {
        assert_eq!(COMMANDS.len(), 114);
        assert_eq!(commands("Backspace"), ["deleteBackward:"]);
        assert_eq!(commands("Meta+KeyA"), ["selectAll:"]);
        assert_eq!(commands("Shift+Meta+KeyZ"), ["redo:"]);
        assert!(commands("KeyA").is_empty());
    }

    #[test]
    fn chromium_drops_inserts_and_colons() {
        assert_eq!(chromium_commands("Backspace", 0), ["deleteBackward"]);
        assert_eq!(chromium_commands("KeyA", META), ["selectAll"]);
        assert!(chromium_commands("Enter", 0).is_empty());
    }

    #[test]
    fn webkit_keeps_them_all() {
        assert_eq!(webkit_commands("Backspace", 0), ["deleteBackward:"]);
        assert_eq!(webkit_commands("Enter", 0), ["insertNewline:"]);
        assert!(webkit_commands("KeyA", 0).is_empty());
    }
}
