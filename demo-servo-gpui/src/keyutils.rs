// Maps GPUI keyboard events to Servo's `keyboard_types`-based KeyboardEvent.

use gpui::{KeyDownEvent, KeyUpEvent, Keystroke, Modifiers as GpuiModifiers};
use servo::{Code, Key, KeyboardEvent, KeyState, Location, Modifiers, NamedKey};

/// Build a Servo [`KeyboardEvent`] from a GPUI key-down event.
pub fn keyboard_event_from_gpui_down(event: &KeyDownEvent) -> KeyboardEvent {
    make_event(&event.keystroke, KeyState::Down, event.is_held)
}

/// Build a Servo [`KeyboardEvent`] from a GPUI key-up event.
pub fn keyboard_event_from_gpui_up(event: &KeyUpEvent) -> KeyboardEvent {
    make_event(&event.keystroke, KeyState::Up, false)
}

fn make_event(ks: &Keystroke, state: KeyState, repeat: bool) -> KeyboardEvent {
    let key = logical_key(ks);
    let code = code_from_key(&ks.key);
    let modifiers = convert_modifiers(&ks.modifiers);

    KeyboardEvent::new_without_event(
        state,
        key,
        code,
        Location::Standard,
        modifiers,
        repeat,
        false,
    )
}

fn convert_modifiers(m: &GpuiModifiers) -> Modifiers {
    let mut out = Modifiers::empty();
    out.set(Modifiers::CONTROL, m.control);
    out.set(Modifiers::SHIFT, m.shift);
    out.set(Modifiers::ALT, m.alt);
    out.set(Modifiers::META, m.platform);
    out
}

// ── Logical key mapping ──────────────────────────────────────────────────────

fn logical_key(ks: &Keystroke) -> Key {
    // When key_char is available and ctrl/alt are not held, use the actual
    // typed character (e.g. shift+a → "A", a → "a").
    if !ks.modifiers.control && !ks.modifiers.alt {
        if let Some(c) = &ks.key_char {
            if !c.is_empty() {
                return Key::Character(c.clone());
            }
        }
    }

    // Map named / special keys by the `key` string.
    named_key_from_str(&ks.key)
}

fn named_key_from_str(key: &str) -> Key {
    Key::Named(match key {
        // Navigation
        "ArrowDown" | "down" => NamedKey::ArrowDown,
        "ArrowLeft" | "left" => NamedKey::ArrowLeft,
        "ArrowRight" | "right" => NamedKey::ArrowRight,
        "ArrowUp" | "up" => NamedKey::ArrowUp,
        "Home" | "home" => NamedKey::Home,
        "End" | "end" => NamedKey::End,
        "PageDown" | "pagedown" | "page_down" => NamedKey::PageDown,
        "PageUp" | "pageup" | "page_up" => NamedKey::PageUp,

        // Editing
        "Backspace" | "backspace" => NamedKey::Backspace,
        "Delete" | "delete" => NamedKey::Delete,
        "Enter" | "enter" | "return" => NamedKey::Enter,
        "Escape" | "escape" => NamedKey::Escape,
        "Insert" | "insert" => NamedKey::Insert,
        "Tab" | "tab" => NamedKey::Tab,
        // "space" / " " handled via key_char path above

        // Modifiers
        "Alt" | "alt" => NamedKey::Alt,
        "AltGraph" | "altgraph" | "alt_graph" => NamedKey::AltGraph,
        "CapsLock" | "caps_lock" => NamedKey::CapsLock,
        "Control" | "control" | "ctrl" => NamedKey::Control,
        "Fn" | "fn" => NamedKey::Fn,
        "FnLock" | "fn_lock" => NamedKey::FnLock,
        "Meta" | "meta" | "cmd" | "command" | "win" => NamedKey::Meta,
        "NumLock" | "numlock" | "num_lock" => NamedKey::NumLock,
        "ScrollLock" | "scroll_lock" => NamedKey::ScrollLock,
        "Shift" | "shift" => NamedKey::Shift,
        "Super" | "super" => NamedKey::Meta,

        // Function keys
        "F1" | "f1" => NamedKey::F1,
        "F2" | "f2" => NamedKey::F2,
        "F3" | "f3" => NamedKey::F3,
        "F4" | "f4" => NamedKey::F4,
        "F5" | "f5" => NamedKey::F5,
        "F6" | "f6" => NamedKey::F6,
        "F7" | "f7" => NamedKey::F7,
        "F8" | "f8" => NamedKey::F8,
        "F9" | "f9" => NamedKey::F9,
        "F10" | "f10" => NamedKey::F10,
        "F11" | "f11" => NamedKey::F11,
        "F12" | "f12" => NamedKey::F12,

        // Media / browser
        "BrowserBack" | "browser_back" => NamedKey::BrowserBack,
        "BrowserForward" | "browser_forward" => NamedKey::BrowserForward,
        "BrowserRefresh" | "browser_refresh" => NamedKey::BrowserRefresh,
        "MediaPlayPause" | "media_play_pause" => NamedKey::MediaPlayPause,
        "AudioVolumeUp" | "volume_up" => NamedKey::AudioVolumeUp,
        "AudioVolumeDown" | "volume_down" => NamedKey::AudioVolumeDown,
        "AudioVolumeMute" | "volume_mute" => NamedKey::AudioVolumeMute,

        _ => NamedKey::Unidentified,
    })
}

// ── Physical code mapping ────────────────────────────────────────────────────

fn code_from_key(key: &str) -> Code {
    match key {
        // Letters
        "a" => Code::KeyA,
        "b" => Code::KeyB,
        "c" => Code::KeyC,
        "d" => Code::KeyD,
        "e" => Code::KeyE,
        "f" => Code::KeyF,
        "g" => Code::KeyG,
        "h" => Code::KeyH,
        "i" => Code::KeyI,
        "j" => Code::KeyJ,
        "k" => Code::KeyK,
        "l" => Code::KeyL,
        "m" => Code::KeyM,
        "n" => Code::KeyN,
        "o" => Code::KeyO,
        "p" => Code::KeyP,
        "q" => Code::KeyQ,
        "r" => Code::KeyR,
        "s" => Code::KeyS,
        "t" => Code::KeyT,
        "u" => Code::KeyU,
        "v" => Code::KeyV,
        "w" => Code::KeyW,
        "x" => Code::KeyX,
        "y" => Code::KeyY,
        "z" => Code::KeyZ,

        // Digits
        "0" => Code::Digit0,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        "3" => Code::Digit3,
        "4" => Code::Digit4,
        "5" => Code::Digit5,
        "6" => Code::Digit6,
        "7" => Code::Digit7,
        "8" => Code::Digit8,
        "9" => Code::Digit9,

        // Editing / control
        "backspace" => Code::Backspace,
        "delete" => Code::Delete,
        "enter" | "return" => Code::Enter,
        "escape" => Code::Escape,
        "insert" => Code::Insert,
        "tab" => Code::Tab,
        "space" | " " => Code::Space,

        // Navigation
        "left" | "ArrowLeft" => Code::ArrowLeft,
        "right" | "ArrowRight" => Code::ArrowRight,
        "up" | "ArrowUp" => Code::ArrowUp,
        "down" | "ArrowDown" => Code::ArrowDown,
        "home" => Code::Home,
        "end" => Code::End,
        "pageup" | "page_up" => Code::PageUp,
        "pagedown" | "page_down" => Code::PageDown,

        // Modifiers
        "shift" => Code::ShiftLeft,
        "control" | "ctrl" => Code::ControlLeft,
        "alt" => Code::AltLeft,
        "meta" | "cmd" | "super" | "win" => Code::MetaLeft,
        "caps_lock" => Code::CapsLock,

        // Punctuation
        "`" => Code::Backquote,
        "-" => Code::Minus,
        "=" => Code::Equal,
        "[" => Code::BracketLeft,
        "]" => Code::BracketRight,
        "\\" => Code::Backslash,
        ";" => Code::Semicolon,
        "'" => Code::Quote,
        "," => Code::Comma,
        "." => Code::Period,
        "/" => Code::Slash,

        // Function keys
        "f1" | "F1" => Code::F1,
        "f2" | "F2" => Code::F2,
        "f3" | "F3" => Code::F3,
        "f4" | "F4" => Code::F4,
        "f5" | "F5" => Code::F5,
        "f6" | "F6" => Code::F6,
        "f7" | "F7" => Code::F7,
        "f8" | "F8" => Code::F8,
        "f9" | "F9" => Code::F9,
        "f10" | "F10" => Code::F10,
        "f11" | "F11" => Code::F11,
        "f12" | "F12" => Code::F12,

        _ => Code::Unidentified,
    }
}
