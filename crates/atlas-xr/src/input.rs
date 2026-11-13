//! Translation of Android keys to [`atlas_editor::Key`].

use android_activity::input::{KeyEvent, KeyMapChar, Keycode};
use android_activity::AndroidApp;
use atlas_editor::Key;

/// Converts a keyboard event into an editor key.
///
/// Control keys are mapped directly. For the rest the device's character
/// map is consulted (it respects the layout and Shift); if the
/// device is virtual, like the one `adb shell input` uses, a basic US map
/// is used instead.
pub fn map_key(app: &AndroidApp, ev: &KeyEvent) -> Option<Key> {
    let code = ev.key_code();
    let special = match code {
        Keycode::Escape => Some(Key::Escape),
        Keycode::Enter | Keycode::NumpadEnter => Some(Key::Enter),
        Keycode::Del => Some(Key::Backspace),
        Keycode::ForwardDel => Some(Key::Delete),
        Keycode::DpadLeft => Some(Key::Left),
        Keycode::DpadRight => Some(Key::Right),
        Keycode::DpadUp => Some(Key::Up),
        Keycode::DpadDown => Some(Key::Down),
        Keycode::MoveHome => Some(Key::Home),
        Keycode::MoveEnd => Some(Key::End),
        Keycode::Tab => Some(Key::Char('\t')),
        _ => None,
    };
    if special.is_some() {
        return special;
    }

    // Ctrl + letter: resolved by key code, because with Ctrl the character
    // map returns control characters.
    // `Cmd`/`Win` is treated as Ctrl, so that Cmd+S saves on Mac keyboards.
    if ev.meta_state().ctrl_on() || ev.meta_state().meta_on() {
        if let Some(c) = us_fallback(code, false).filter(char::is_ascii_alphabetic) {
            return Some(Key::Ctrl(c));
        }
    }

    if let Ok(map) = app.device_key_character_map(ev.device_id()) {
        if let Ok(KeyMapChar::Unicode(c)) = map.get(code, ev.meta_state()) {
            return Some(Key::Char(c));
        }
    }
    us_fallback(code, ev.meta_state().shift_on()).map(Key::Char)
}

/// Minimal US map by key code (Android `KEYCODE_*` values).
fn us_fallback(code: Keycode, shift: bool) -> Option<char> {
    let n = u32::from(code);
    let c = match n {
        29..=54 => {
            let c = (b'a' + (n - 29) as u8) as char;
            if shift { c.to_ascii_uppercase() } else { c }
        }
        7..=16 => {
            let i = (n - 7) as usize;
            if shift { ")!@#$%^&*(".chars().nth(i)? } else { (b'0' + i as u8) as char }
        }
        62 => ' ',
        55 => if shift { '<' } else { ',' },
        56 => if shift { '>' } else { '.' },
        69 => if shift { '_' } else { '-' },
        70 => if shift { '+' } else { '=' },
        71 => if shift { '{' } else { '[' },
        72 => if shift { '}' } else { ']' },
        73 => if shift { '|' } else { '\\' },
        74 => if shift { ':' } else { ';' },
        75 => if shift { '"' } else { '\'' },
        76 => if shift { '?' } else { '/' },
        _ => return None,
    };
    Some(c)
}
