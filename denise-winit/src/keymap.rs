//! winit key and modifier translation.

use denise::{KeyCode, Modifiers};
use winit::keyboard::{ModifiersState, PhysicalKey};

/// Maps a winit physical key to a Denise [`KeyCode`].
///
/// Unmapped positions become [`KeyCode::Unidentified`]. winit hides the native
/// scancode behind a platform-specific extension trait, so we pass `0` rather than
/// pretend; `denise-evdev` will supply real codes in M2.
pub fn key_code(key: PhysicalKey) -> KeyCode {
    use winit::keyboard::KeyCode as W;
    let PhysicalKey::Code(code) = key else {
        return KeyCode::Unidentified(0);
    };
    match code {
        W::KeyA => KeyCode::A,
        W::KeyB => KeyCode::B,
        W::KeyC => KeyCode::C,
        W::KeyD => KeyCode::D,
        W::KeyE => KeyCode::E,
        W::KeyF => KeyCode::F,
        W::KeyG => KeyCode::G,
        W::KeyH => KeyCode::H,
        W::KeyI => KeyCode::I,
        W::KeyJ => KeyCode::J,
        W::KeyK => KeyCode::K,
        W::KeyL => KeyCode::L,
        W::KeyM => KeyCode::M,
        W::KeyN => KeyCode::N,
        W::KeyO => KeyCode::O,
        W::KeyP => KeyCode::P,
        W::KeyQ => KeyCode::Q,
        W::KeyR => KeyCode::R,
        W::KeyS => KeyCode::S,
        W::KeyT => KeyCode::T,
        W::KeyU => KeyCode::U,
        W::KeyV => KeyCode::V,
        W::KeyW => KeyCode::W,
        W::KeyX => KeyCode::X,
        W::KeyY => KeyCode::Y,
        W::KeyZ => KeyCode::Z,

        W::Digit0 => KeyCode::Digit0,
        W::Digit1 => KeyCode::Digit1,
        W::Digit2 => KeyCode::Digit2,
        W::Digit3 => KeyCode::Digit3,
        W::Digit4 => KeyCode::Digit4,
        W::Digit5 => KeyCode::Digit5,
        W::Digit6 => KeyCode::Digit6,
        W::Digit7 => KeyCode::Digit7,
        W::Digit8 => KeyCode::Digit8,
        W::Digit9 => KeyCode::Digit9,

        W::F1 => KeyCode::F1,
        W::F2 => KeyCode::F2,
        W::F3 => KeyCode::F3,
        W::F4 => KeyCode::F4,
        W::F5 => KeyCode::F5,
        W::F6 => KeyCode::F6,
        W::F7 => KeyCode::F7,
        W::F8 => KeyCode::F8,
        W::F9 => KeyCode::F9,
        W::F10 => KeyCode::F10,
        W::F11 => KeyCode::F11,
        W::F12 => KeyCode::F12,

        W::Escape => KeyCode::Escape,
        W::Enter => KeyCode::Enter,
        W::Tab => KeyCode::Tab,
        W::Space => KeyCode::Space,
        W::Backspace => KeyCode::Backspace,
        W::Delete => KeyCode::Delete,
        W::Insert => KeyCode::Insert,
        W::Home => KeyCode::Home,
        W::End => KeyCode::End,
        W::PageUp => KeyCode::PageUp,
        W::PageDown => KeyCode::PageDown,
        W::ArrowUp => KeyCode::ArrowUp,
        W::ArrowDown => KeyCode::ArrowDown,
        W::ArrowLeft => KeyCode::ArrowLeft,
        W::ArrowRight => KeyCode::ArrowRight,

        W::ShiftLeft => KeyCode::ShiftLeft,
        W::ShiftRight => KeyCode::ShiftRight,
        W::ControlLeft => KeyCode::ControlLeft,
        W::ControlRight => KeyCode::ControlRight,
        W::AltLeft => KeyCode::AltLeft,
        W::AltRight => KeyCode::AltRight,
        W::SuperLeft => KeyCode::SuperLeft,
        W::SuperRight => KeyCode::SuperRight,
        W::CapsLock => KeyCode::CapsLock,
        W::NumLock => KeyCode::NumLock,
        W::ScrollLock => KeyCode::ScrollLock,

        W::Minus => KeyCode::Minus,
        W::Equal => KeyCode::Equal,
        W::BracketLeft => KeyCode::BracketLeft,
        W::BracketRight => KeyCode::BracketRight,
        W::Backslash => KeyCode::Backslash,
        W::Semicolon => KeyCode::Semicolon,
        W::Quote => KeyCode::Quote,
        W::Backquote => KeyCode::Backquote,
        W::Comma => KeyCode::Comma,
        W::Period => KeyCode::Period,
        W::Slash => KeyCode::Slash,

        W::NumpadEnter => KeyCode::NumpadEnter,
        W::NumpadAdd => KeyCode::NumpadAdd,
        W::NumpadSubtract => KeyCode::NumpadSubtract,
        W::NumpadMultiply => KeyCode::NumpadMultiply,
        W::NumpadDivide => KeyCode::NumpadDivide,
        W::NumpadDecimal => KeyCode::NumpadDecimal,
        W::Numpad0 => KeyCode::Numpad0,
        W::Numpad1 => KeyCode::Numpad1,
        W::Numpad2 => KeyCode::Numpad2,
        W::Numpad3 => KeyCode::Numpad3,
        W::Numpad4 => KeyCode::Numpad4,
        W::Numpad5 => KeyCode::Numpad5,
        W::Numpad6 => KeyCode::Numpad6,
        W::Numpad7 => KeyCode::Numpad7,
        W::Numpad8 => KeyCode::Numpad8,
        W::Numpad9 => KeyCode::Numpad9,

        _ => KeyCode::Unidentified(0),
    }
}

/// Maps winit's modifier state to Denise's.
pub fn modifiers(state: ModifiersState) -> Modifiers {
    Modifiers::NONE
        .set(Modifiers::SHIFT, state.shift_key())
        .set(Modifiers::CTRL, state.control_key())
        .set(Modifiers::ALT, state.alt_key())
        .set(Modifiers::SUPER, state.super_key())
}
