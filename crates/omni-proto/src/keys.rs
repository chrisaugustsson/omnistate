//! Key names: how a `keyPress` trigger writes a key, a combination or a
//! controller button, and how the player compares what was pressed with it.
//!
//! Ported from FigCraft's `KeyMods` (MIT OR Apache-2.0; see `NOTICE`).

/// The modifiers held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

/// The buttons of a controller with the browser's standard layout, by index:
/// `Gamepad A` is button 0, `Gamepad LB` button 4, `Gamepad Up` button 12.
pub const GAMEPAD_BUTTONS: [&str; 17] = [
    "A", "B", "X", "Y", "LB", "RB", "LT", "RT", "Back", "Start", "L3", "R3", "Up", "Down", "Left",
    "Right", "Home",
];

/// The trigger text for `key` held with `mods`: the names `Ctrl`, `Alt`,
/// `Shift` and `Cmd`, in that order, then the key, joined with `+`.
pub fn combo(mods: Mods, key: &str) -> String {
    let mut text = String::new();
    for (held, name) in [
        (mods.ctrl, "Ctrl"),
        (mods.alt, "Alt"),
        (mods.shift, "Shift"),
        (mods.cmd, "Cmd"),
    ] {
        if held {
            text.push_str(name);
            text.push('+');
        }
    }
    text.push_str(key);
    text
}

/// The modifiers and the key of a trigger's text, whatever the order and case
/// of the modifiers. The key is `key_alias`ed. `Shift++` is Shift and the
/// key `+`.
pub fn split(text: &str) -> (Mods, String) {
    let mut mods = Mods::default();
    let mut rest = text.trim();
    while let Some((head, tail)) = rest.split_once('+') {
        if tail.is_empty() {
            break;
        }
        let held = match head.trim().to_lowercase().as_str() {
            "ctrl" | "control" => &mut mods.ctrl,
            "alt" | "option" => &mut mods.alt,
            "shift" => &mut mods.shift,
            "cmd" | "meta" | "command" => &mut mods.cmd,
            _ => break,
        };
        *held = true;
        rest = tail.trim_start();
    }
    (mods, key_alias(rest))
}

/// A key's name in the one spelling the player compares: lower case, with
/// `" "` as `space`, and the other names a keyboard event or a trigger may
/// use for a key as the one `KeyboardEvent.key` gives.
pub fn key_alias(key: &str) -> String {
    if key == " " {
        return "space".to_string();
    }
    let key = key.trim().to_lowercase();
    match key.as_str() {
        "spacebar" => "space".to_string(),
        "esc" => "escape".to_string(),
        "return" => "enter".to_string(),
        "del" => "delete".to_string(),
        "left" | "right" | "up" | "down" => format!("arrow{key}"),
        _ => key,
    }
}

/// The name of the controller button at `index`, if the layout has one.
pub fn gamepad_button(index: usize) -> Option<&'static str> {
    GAMEPAD_BUTTONS.get(index).copied()
}
