// Maps egui keyboard input to Servo's `keyboard_types`-based KeyboardEvent.
//
// egui splits keyboard input into `Event::Key` (named/physical keys) and
// `Event::Text` (the typed, layout- and shift-resolved characters). To avoid
// double-input, this module forwards only NON-text named keys via `Event::Key`
// (Enter, Tab, arrows, editing keys, function keys, clipboard keys), and lets
// typed characters arrive via `Event::Text` as `Key::Character`.

use eframe::egui;
use servo::{Code, Key, KeyState, KeyboardEvent, Location, Modifiers, NamedKey};

/// Build a Servo [`KeyboardEvent`] for a named (non-text) egui key.
///
/// Returns `None` for keys whose character is better delivered via egui's
/// `Event::Text` (letters, digits, punctuation, space) so input isn't doubled.
pub fn named_key_event(
    key: egui::Key,
    pressed: bool,
    modifiers: egui::Modifiers,
    repeat: bool,
) -> Option<KeyboardEvent> {
    let named = map_named(key)?;
    Some(KeyboardEvent::new_without_event(
        if pressed { KeyState::Down } else { KeyState::Up },
        Key::Named(named),
        map_code(key),
        Location::Standard,
        convert_modifiers(modifiers),
        repeat,
        false,
    ))
}

/// Build the Down+Up [`KeyboardEvent`] pair for a typed character string from
/// egui's `Event::Text`.
pub fn text_key_events(ch: &str, modifiers: egui::Modifiers) -> [KeyboardEvent; 2] {
    let mods = convert_modifiers(modifiers);
    let make = |state| {
        KeyboardEvent::new_without_event(
            state,
            Key::Character(ch.to_string()),
            Code::Unidentified,
            Location::Standard,
            mods,
            false,
            false,
        )
    };
    [make(KeyState::Down), make(KeyState::Up)]
}

fn convert_modifiers(m: egui::Modifiers) -> Modifiers {
    let mut out = Modifiers::empty();
    out.set(Modifiers::CONTROL, m.ctrl);
    out.set(Modifiers::SHIFT, m.shift);
    out.set(Modifiers::ALT, m.alt);
    // egui only surfaces the Cmd key as a distinct modifier on macOS; on other
    // platforms the OS "super" key is folded into `command`/`ctrl`, so META is
    // best-effort mapped from `mac_cmd` only.
    out.set(Modifiers::META, m.mac_cmd);
    out
}

/// Map an egui named key to a W3C NamedKey, or `None` for text-producing keys.
fn map_named(key: egui::Key) -> Option<NamedKey> {
    use egui::Key as K;
    Some(match key {
        // Navigation
        K::ArrowDown => NamedKey::ArrowDown,
        K::ArrowLeft => NamedKey::ArrowLeft,
        K::ArrowRight => NamedKey::ArrowRight,
        K::ArrowUp => NamedKey::ArrowUp,
        K::Home => NamedKey::Home,
        K::End => NamedKey::End,
        K::PageDown => NamedKey::PageDown,
        K::PageUp => NamedKey::PageUp,

        // Whitespace / editing (Space intentionally omitted: arrives as Text)
        K::Backspace => NamedKey::Backspace,
        K::Delete => NamedKey::Delete,
        K::Enter => NamedKey::Enter,
        K::Escape => NamedKey::Escape,
        K::Insert => NamedKey::Insert,
        K::Tab => NamedKey::Tab,

        // Clipboard
        K::Copy => NamedKey::Copy,
        K::Cut => NamedKey::Cut,
        K::Paste => NamedKey::Paste,

        // Function keys
        K::F1 => NamedKey::F1,
        K::F2 => NamedKey::F2,
        K::F3 => NamedKey::F3,
        K::F4 => NamedKey::F4,
        K::F5 => NamedKey::F5,
        K::F6 => NamedKey::F6,
        K::F7 => NamedKey::F7,
        K::F8 => NamedKey::F8,
        K::F9 => NamedKey::F9,
        K::F10 => NamedKey::F10,
        K::F11 => NamedKey::F11,
        K::F12 => NamedKey::F12,

        // Letters, digits, punctuation → delivered via Event::Text.
        _ => return None,
    })
}

/// Best-effort physical [`Code`] for a named egui key.
fn map_code(key: egui::Key) -> Code {
    use egui::Key as K;
    match key {
        K::ArrowDown => Code::ArrowDown,
        K::ArrowLeft => Code::ArrowLeft,
        K::ArrowRight => Code::ArrowRight,
        K::ArrowUp => Code::ArrowUp,
        K::Home => Code::Home,
        K::End => Code::End,
        K::PageDown => Code::PageDown,
        K::PageUp => Code::PageUp,
        K::Backspace => Code::Backspace,
        K::Delete => Code::Delete,
        K::Enter => Code::Enter,
        K::Escape => Code::Escape,
        K::Insert => Code::Insert,
        K::Tab => Code::Tab,
        K::F1 => Code::F1,
        K::F2 => Code::F2,
        K::F3 => Code::F3,
        K::F4 => Code::F4,
        K::F5 => Code::F5,
        K::F6 => Code::F6,
        K::F7 => Code::F7,
        K::F8 => Code::F8,
        K::F9 => Code::F9,
        K::F10 => Code::F10,
        K::F11 => Code::F11,
        K::F12 => Code::F12,
        _ => Code::Unidentified,
    }
}
