// Maps iced keyboard events to Servo's `keyboard_types`-based KeyboardEvent.
// Both iced and keyboard_types follow the W3C UI Events specification,
// so the mapping is 1:1 for named keys and physical key codes.

use iced::keyboard::{self, key};
use servo::{Code, Key, KeyState, KeyboardEvent, Location, Modifiers, NamedKey};

/// Build a Servo [`KeyboardEvent`] from an iced keyboard event.
pub fn keyboard_event_from_iced(
    state: KeyState,
    iced_key: &keyboard::Key,
    physical_key: key::Physical,
    iced_location: keyboard::Location,
    iced_modifiers: keyboard::Modifiers,
    repeat: bool,
) -> KeyboardEvent {
    KeyboardEvent::new_without_event(
        state,
        logical_key(iced_key),
        physical_code(physical_key),
        location(iced_location),
        convert_modifiers(iced_modifiers),
        repeat,
        false,
    )
}

fn convert_modifiers(m: keyboard::Modifiers) -> Modifiers {
    let mut out = Modifiers::empty();
    out.set(Modifiers::CONTROL, m.control());
    out.set(Modifiers::SHIFT, m.shift());
    out.set(Modifiers::ALT, m.alt());
    out.set(Modifiers::META, m.logo());
    out
}

fn location(loc: keyboard::Location) -> Location {
    match loc {
        keyboard::Location::Standard => Location::Standard,
        keyboard::Location::Left => Location::Left,
        keyboard::Location::Right => Location::Right,
        keyboard::Location::Numpad => Location::Numpad,
    }
}

// ── Logical key mapping ──────────────────────────────────────────────────────

fn logical_key(k: &keyboard::Key) -> Key {
    match k {
        keyboard::Key::Character(s) => Key::Character(s.to_string()),
        keyboard::Key::Unidentified => Key::Named(NamedKey::Unidentified),
        keyboard::Key::Named(n) => named_key(*n),
    }
}

/// Map iced Named → keyboard_types NamedKey.
/// Both follow the W3C UI Events spec, so variant names match exactly.
#[allow(deprecated)]
fn named_key(k: key::Named) -> Key {
    use key::Named as N;
    Key::Named(match k {
        // Navigation
        N::ArrowDown => NamedKey::ArrowDown,
        N::ArrowLeft => NamedKey::ArrowLeft,
        N::ArrowRight => NamedKey::ArrowRight,
        N::ArrowUp => NamedKey::ArrowUp,
        N::Home => NamedKey::Home,
        N::End => NamedKey::End,
        N::PageDown => NamedKey::PageDown,
        N::PageUp => NamedKey::PageUp,

        // Whitespace / editing
        N::Backspace => NamedKey::Backspace,
        N::Delete => NamedKey::Delete,
        N::Enter => NamedKey::Enter,
        N::Escape => NamedKey::Escape,
        N::Insert => NamedKey::Insert,
        N::Tab => NamedKey::Tab,
        N::Space => return Key::Character(" ".to_string()),

        // Modifiers
        N::Alt => NamedKey::Alt,
        N::AltGraph => NamedKey::AltGraph,
        N::CapsLock => NamedKey::CapsLock,
        N::Control => NamedKey::Control,
        N::Fn => NamedKey::Fn,
        N::FnLock => NamedKey::FnLock,
        N::Meta => NamedKey::Meta,
        N::NumLock => NamedKey::NumLock,
        N::ScrollLock => NamedKey::ScrollLock,
        N::Shift => NamedKey::Shift,
        N::Super => NamedKey::Super,
        N::Hyper => NamedKey::Hyper,
        N::Symbol => NamedKey::Symbol,
        N::SymbolLock => NamedKey::SymbolLock,

        // Function keys
        N::F1 => NamedKey::F1,
        N::F2 => NamedKey::F2,
        N::F3 => NamedKey::F3,
        N::F4 => NamedKey::F4,
        N::F5 => NamedKey::F5,
        N::F6 => NamedKey::F6,
        N::F7 => NamedKey::F7,
        N::F8 => NamedKey::F8,
        N::F9 => NamedKey::F9,
        N::F10 => NamedKey::F10,
        N::F11 => NamedKey::F11,
        N::F12 => NamedKey::F12,
        N::F13 => NamedKey::F13,
        N::F14 => NamedKey::F14,
        N::F15 => NamedKey::F15,
        N::F16 => NamedKey::F16,
        N::F17 => NamedKey::F17,
        N::F18 => NamedKey::F18,
        N::F19 => NamedKey::F19,
        N::F20 => NamedKey::F20,

        // IME / compose
        N::Compose => NamedKey::Compose,
        N::Convert => NamedKey::Convert,
        N::NonConvert => NamedKey::NonConvert,
        N::KanaMode => NamedKey::KanaMode,
        N::KanjiMode => NamedKey::KanjiMode,

        // Browser
        N::BrowserBack => NamedKey::BrowserBack,
        N::BrowserForward => NamedKey::BrowserForward,
        N::BrowserRefresh => NamedKey::BrowserRefresh,
        N::BrowserStop => NamedKey::BrowserStop,
        N::BrowserSearch => NamedKey::BrowserSearch,
        N::BrowserFavorites => NamedKey::BrowserFavorites,
        N::BrowserHome => NamedKey::BrowserHome,

        // Media
        N::MediaPlayPause => NamedKey::MediaPlayPause,
        N::MediaStop => NamedKey::MediaStop,
        N::MediaTrackNext => NamedKey::MediaTrackNext,
        N::MediaTrackPrevious => NamedKey::MediaTrackPrevious,
        N::AudioVolumeDown => NamedKey::AudioVolumeDown,
        N::AudioVolumeMute => NamedKey::AudioVolumeMute,
        N::AudioVolumeUp => NamedKey::AudioVolumeUp,

        // Editing
        N::Copy => NamedKey::Copy,
        N::Cut => NamedKey::Cut,
        N::Paste => NamedKey::Paste,
        N::Undo => NamedKey::Undo,
        N::Redo => NamedKey::Redo,
        N::Find => NamedKey::Find,
        N::Select => NamedKey::Select,

        // UI
        N::ContextMenu => NamedKey::ContextMenu,
        N::Help => NamedKey::Help,
        N::Pause => NamedKey::Pause,
        N::PrintScreen => NamedKey::PrintScreen,

        // Misc
        N::Clear => NamedKey::Clear,
        N::Cancel => NamedKey::Cancel,
        N::Accept => NamedKey::Accept,
        N::Execute => NamedKey::Execute,
        N::Play => NamedKey::Play,
        N::ZoomIn => NamedKey::ZoomIn,
        N::ZoomOut => NamedKey::ZoomOut,

        _ => NamedKey::Unidentified,
    })
}

// ── Physical key code mapping ────────────────────────────────────────────────

fn physical_code(physical: key::Physical) -> Code {
    let kc = match physical {
        key::Physical::Code(kc) => kc,
        key::Physical::Unidentified(_) => return Code::Unidentified,
    };
    use key::Code as K;
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
