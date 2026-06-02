// Adapted from Servo's servoshell/desktop/keyutils.rs
// Original: Mozilla Public License 2.0
//
// Maps winit keyboard events to Servo's `keyboard_types`-based KeyboardEvent.
// Both crates follow the W3C UI Events specification, so the mapping is 1:1
// for named keys and physical key codes.

use servo::{Code, Key, KeyState, KeyboardEvent, Location, Modifiers, NamedKey};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{
    Key as WinitKey, KeyCode, KeyLocation, ModifiersState, NamedKey as WinitNamedKey, PhysicalKey,
};

/// Build a Servo [`KeyboardEvent`] from a winit [`KeyEvent`] plus current modifier state.
pub fn keyboard_event_from_winit(key_event: &KeyEvent, modifiers: ModifiersState) -> KeyboardEvent {
    KeyboardEvent::new_without_event(
        key_state(key_event),
        logical_key(key_event),
        physical_code(key_event),
        location(key_event),
        convert_modifiers(modifiers),
        key_event.repeat,
        false,
    )
}

fn key_state(e: &KeyEvent) -> KeyState {
    match e.state {
        ElementState::Pressed => KeyState::Down,
        ElementState::Released => KeyState::Up,
    }
}

fn convert_modifiers(m: ModifiersState) -> Modifiers {
    let mut out = Modifiers::empty();
    out.set(Modifiers::CONTROL, m.control_key());
    out.set(Modifiers::SHIFT, m.shift_key());
    out.set(Modifiers::ALT, m.alt_key());
    out.set(Modifiers::META, m.super_key());
    out
}

fn location(e: &KeyEvent) -> Location {
    match e.location {
        KeyLocation::Standard => Location::Standard,
        KeyLocation::Left => Location::Left,
        KeyLocation::Right => Location::Right,
        KeyLocation::Numpad => Location::Numpad,
    }
}

// ── Logical key mapping ──────────────────────────────────────────────────────

fn logical_key(e: &KeyEvent) -> Key {
    match &e.logical_key {
        WinitKey::Character(s) => Key::Character(s.to_string()),
        WinitKey::Dead(_) | WinitKey::Unidentified(_) => Key::Named(NamedKey::Unidentified),
        WinitKey::Named(n) => named_key(*n),
    }
}

/// Map winit NamedKey → keyboard_types NamedKey.
/// Both follow the W3C UI Events spec, so variant names match exactly.
#[allow(deprecated)] // W3C legacy keys (Super, Hyper) are deprecated in keyboard_types
fn named_key(k: WinitNamedKey) -> Key {
    use WinitNamedKey as W;
    Key::Named(match k {
        // Navigation
        W::ArrowDown => NamedKey::ArrowDown,
        W::ArrowLeft => NamedKey::ArrowLeft,
        W::ArrowRight => NamedKey::ArrowRight,
        W::ArrowUp => NamedKey::ArrowUp,
        W::Home => NamedKey::Home,
        W::End => NamedKey::End,
        W::PageDown => NamedKey::PageDown,
        W::PageUp => NamedKey::PageUp,

        // Whitespace / editing
        W::Backspace => NamedKey::Backspace,
        W::Delete => NamedKey::Delete,
        W::Enter => NamedKey::Enter,
        W::Escape => NamedKey::Escape,
        W::Insert => NamedKey::Insert,
        W::Tab => NamedKey::Tab,
        W::Space => return Key::Character(" ".to_string()),

        // Modifiers
        W::Alt => NamedKey::Alt,
        W::AltGraph => NamedKey::AltGraph,
        W::CapsLock => NamedKey::CapsLock,
        W::Control => NamedKey::Control,
        W::Fn => NamedKey::Fn,
        W::FnLock => NamedKey::FnLock,
        W::Meta => NamedKey::Meta,
        W::NumLock => NamedKey::NumLock,
        W::ScrollLock => NamedKey::ScrollLock,
        W::Shift => NamedKey::Shift,
        W::Super => NamedKey::Super,
        W::Hyper => NamedKey::Hyper,
        W::Symbol => NamedKey::Symbol,
        W::SymbolLock => NamedKey::SymbolLock,

        // Function keys
        W::F1 => NamedKey::F1,
        W::F2 => NamedKey::F2,
        W::F3 => NamedKey::F3,
        W::F4 => NamedKey::F4,
        W::F5 => NamedKey::F5,
        W::F6 => NamedKey::F6,
        W::F7 => NamedKey::F7,
        W::F8 => NamedKey::F8,
        W::F9 => NamedKey::F9,
        W::F10 => NamedKey::F10,
        W::F11 => NamedKey::F11,
        W::F12 => NamedKey::F12,
        W::F13 => NamedKey::F13,
        W::F14 => NamedKey::F14,
        W::F15 => NamedKey::F15,
        W::F16 => NamedKey::F16,
        W::F17 => NamedKey::F17,
        W::F18 => NamedKey::F18,
        W::F19 => NamedKey::F19,
        W::F20 => NamedKey::F20,

        // IME / compose
        W::Compose => NamedKey::Compose,
        W::Convert => NamedKey::Convert,
        W::NonConvert => NamedKey::NonConvert,
        W::KanaMode => NamedKey::KanaMode,
        W::KanjiMode => NamedKey::KanjiMode,

        // Browser
        W::BrowserBack => NamedKey::BrowserBack,
        W::BrowserForward => NamedKey::BrowserForward,
        W::BrowserRefresh => NamedKey::BrowserRefresh,
        W::BrowserStop => NamedKey::BrowserStop,
        W::BrowserSearch => NamedKey::BrowserSearch,
        W::BrowserFavorites => NamedKey::BrowserFavorites,
        W::BrowserHome => NamedKey::BrowserHome,

        // Media
        W::MediaPlayPause => NamedKey::MediaPlayPause,
        W::MediaStop => NamedKey::MediaStop,
        W::MediaTrackNext => NamedKey::MediaTrackNext,
        W::MediaTrackPrevious => NamedKey::MediaTrackPrevious,
        W::AudioVolumeDown => NamedKey::AudioVolumeDown,
        W::AudioVolumeMute => NamedKey::AudioVolumeMute,
        W::AudioVolumeUp => NamedKey::AudioVolumeUp,

        // Editing
        W::Copy => NamedKey::Copy,
        W::Cut => NamedKey::Cut,
        W::Paste => NamedKey::Paste,
        W::Undo => NamedKey::Undo,
        W::Redo => NamedKey::Redo,
        W::Find => NamedKey::Find,
        W::Select => NamedKey::Select,

        // UI
        W::ContextMenu => NamedKey::ContextMenu,
        W::Help => NamedKey::Help,
        W::Pause => NamedKey::Pause,
        W::PrintScreen => NamedKey::PrintScreen,

        // Misc
        W::Clear => NamedKey::Clear,
        W::Cancel => NamedKey::Cancel,
        W::Accept => NamedKey::Accept,
        W::Execute => NamedKey::Execute,
        W::Play => NamedKey::Play,
        W::Print => NamedKey::Print,
        W::Save => NamedKey::Save,
        W::ZoomIn => NamedKey::ZoomIn,
        W::ZoomOut => NamedKey::ZoomOut,

        _ => NamedKey::Unidentified,
    })
}

// ── Physical key code mapping ────────────────────────────────────────────────

#[allow(deprecated)]
fn physical_code(e: &KeyEvent) -> Code {
    let kc = match e.physical_key {
        PhysicalKey::Code(kc) => kc,
        PhysicalKey::Unidentified(_) => return Code::Unidentified,
    };
    use KeyCode as K;
    match kc {
        // Letters
        K::KeyA => Code::KeyA,
        K::KeyB => Code::KeyB,
        K::KeyC => Code::KeyC,
        K::KeyD => Code::KeyD,
        K::KeyE => Code::KeyE,
        K::KeyF => Code::KeyF,
        K::KeyG => Code::KeyG,
        K::KeyH => Code::KeyH,
        K::KeyI => Code::KeyI,
        K::KeyJ => Code::KeyJ,
        K::KeyK => Code::KeyK,
        K::KeyL => Code::KeyL,
        K::KeyM => Code::KeyM,
        K::KeyN => Code::KeyN,
        K::KeyO => Code::KeyO,
        K::KeyP => Code::KeyP,
        K::KeyQ => Code::KeyQ,
        K::KeyR => Code::KeyR,
        K::KeyS => Code::KeyS,
        K::KeyT => Code::KeyT,
        K::KeyU => Code::KeyU,
        K::KeyV => Code::KeyV,
        K::KeyW => Code::KeyW,
        K::KeyX => Code::KeyX,
        K::KeyY => Code::KeyY,
        K::KeyZ => Code::KeyZ,

        // Digits
        K::Digit0 => Code::Digit0,
        K::Digit1 => Code::Digit1,
        K::Digit2 => Code::Digit2,
        K::Digit3 => Code::Digit3,
        K::Digit4 => Code::Digit4,
        K::Digit5 => Code::Digit5,
        K::Digit6 => Code::Digit6,
        K::Digit7 => Code::Digit7,
        K::Digit8 => Code::Digit8,
        K::Digit9 => Code::Digit9,

        // Numpad
        K::Numpad0 => Code::Numpad0,
        K::Numpad1 => Code::Numpad1,
        K::Numpad2 => Code::Numpad2,
        K::Numpad3 => Code::Numpad3,
        K::Numpad4 => Code::Numpad4,
        K::Numpad5 => Code::Numpad5,
        K::Numpad6 => Code::Numpad6,
        K::Numpad7 => Code::Numpad7,
        K::Numpad8 => Code::Numpad8,
        K::Numpad9 => Code::Numpad9,
        K::NumpadAdd => Code::NumpadAdd,
        K::NumpadDecimal => Code::NumpadDecimal,
        K::NumpadDivide => Code::NumpadDivide,
        K::NumpadEnter => Code::NumpadEnter,
        K::NumpadEqual => Code::NumpadEqual,
        K::NumpadMultiply => Code::NumpadMultiply,
        K::NumpadSubtract => Code::NumpadSubtract,

        // Punctuation / symbols
        K::Backquote => Code::Backquote,
        K::Backslash => Code::Backslash,
        K::BracketLeft => Code::BracketLeft,
        K::BracketRight => Code::BracketRight,
        K::Comma => Code::Comma,
        K::Equal => Code::Equal,
        K::Minus => Code::Minus,
        K::Period => Code::Period,
        K::Quote => Code::Quote,
        K::Semicolon => Code::Semicolon,
        K::Slash => Code::Slash,
        K::IntlBackslash => Code::IntlBackslash,

        // Whitespace / editing
        K::Backspace => Code::Backspace,
        K::Delete => Code::Delete,
        K::Enter => Code::Enter,
        K::Escape => Code::Escape,
        K::Insert => Code::Insert,
        K::Space => Code::Space,
        K::Tab => Code::Tab,

        // Navigation
        K::ArrowDown => Code::ArrowDown,
        K::ArrowLeft => Code::ArrowLeft,
        K::ArrowRight => Code::ArrowRight,
        K::ArrowUp => Code::ArrowUp,
        K::End => Code::End,
        K::Home => Code::Home,
        K::PageDown => Code::PageDown,
        K::PageUp => Code::PageUp,

        // Modifiers
        K::AltLeft => Code::AltLeft,
        K::AltRight => Code::AltRight,
        K::CapsLock => Code::CapsLock,
        K::ControlLeft => Code::ControlLeft,
        K::ControlRight => Code::ControlRight,
        K::ShiftLeft => Code::ShiftLeft,
        K::ShiftRight => Code::ShiftRight,
        K::SuperLeft => Code::MetaLeft,
        K::SuperRight => Code::MetaRight,
        K::NumLock => Code::NumLock,
        K::ScrollLock => Code::ScrollLock,

        // Function keys
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
        K::F13 => Code::F13,
        K::F14 => Code::F14,
        K::F15 => Code::F15,
        K::F16 => Code::F16,
        K::F17 => Code::F17,
        K::F18 => Code::F18,
        K::F19 => Code::F19,
        K::F20 => Code::F20,

        // Misc
        K::ContextMenu => Code::ContextMenu,
        K::PrintScreen => Code::PrintScreen,
        K::Pause => Code::Pause,
        K::Power => Code::Power,

        _ => Code::Unidentified,
    }
}
