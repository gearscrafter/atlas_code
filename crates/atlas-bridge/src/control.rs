//! Obeying the headset: moving the mouse, clicking, turning the wheel and typing on the laptop. It only happens if the
//! user allows it in Atlas Bridge (the box starts unticked).
//!
//! On a Mac, Atlas Bridge needs the "Accessibility" permission (System Settings > Privacy & Security) the
//! first time, and Ctrl becomes Cmd (Cmd+S saves, like Ctrl+S on Windows).

use atlas_link::RemoteInput;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};

/// Longest text accepted in a single order.
const MAX_TEXT: usize = 1000;

pub struct Controller {
    enigo: Enigo,
    /// Buttons (left, right) the headset left pressed: they are released at the end.
    held: [bool; 2],
}

/// The position in pixels of (`u`, `v`) on a `w` x `h` screen (u and v between 0 and 1).
pub fn to_pixels(u: f32, v: f32, w: i32, h: i32) -> (i32, i32) {
    let x = (u.clamp(0.0, 1.0) * (w - 1).max(0) as f32).round() as i32;
    let y = (v.clamp(0.0, 1.0) * (h - 1).max(0) as f32).round() as i32;
    (x, y)
}

/// The key for `name` (`enter`, `backspace`... or a single character).
pub fn key_for(name: &str) -> Option<Key> {
    Some(match name {
        "enter" => Key::Return,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "escape" => Key::Escape,
        "tab" => Key::Tab,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "home" => Key::Home,
        "end" => Key::End,
        other => {
            let mut chars = other.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Key::Unicode(c)
        }
    })
}

impl Controller {
    pub fn new() -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default()).map_err(|e| format!("control: {e}"))?;
        Ok(Self { enigo, held: [false; 2] })
    }

    /// Does what the headset asks.
    pub fn apply(&mut self, input: &RemoteInput) -> Result<(), String> {
        let fail = |e: enigo::InputError| format!("control: {e}");
        let e = &mut self.enigo;
        match input {
            RemoteInput::Pointer { u, v } => {
                // With the size of the main screen as enigo itself sees it: that way it matches the display scaling.
                let (w, h) = e.main_display().map_err(fail)?;
                let (x, y) = to_pixels(*u, *v, w, h);
                e.move_mouse(x, y, Coordinate::Abs).map_err(fail)
            }
            RemoteInput::Button { down, right } => {
                self.held[usize::from(*right)] = *down;
                let button = if *right { Button::Right } else { Button::Left };
                e.button(button, if *down { Direction::Press } else { Direction::Release }).map_err(fail)
            }
            RemoteInput::Scroll { lines } => e.scroll((*lines).clamp(-20, 20), Axis::Vertical).map_err(fail),
            RemoteInput::Text { text } => {
                let text: String = text.chars().filter(|c| !c.is_control()).take(MAX_TEXT).collect();
                if text.is_empty() {
                    Ok(())
                } else {
                    e.text(&text).map_err(fail)
                }
            }
            RemoteInput::Key { key, ctrl } => {
                let Some(k) = key_for(key) else {
                    return Err(format!("control: unknown key {key:?}"));
                };
                let modifier = if cfg!(target_os = "macos") { Key::Meta } else { Key::Control };
                if *ctrl {
                    e.key(modifier, Direction::Press).map_err(fail)?;
                }
                let result = e.key(k, Direction::Click).map_err(fail);
                if *ctrl {
                    let _ = e.key(modifier, Direction::Release);
                }
                result
            }
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        // So that no button stays pressed if the connection drops in the middle of a drag.
        for (i, button) in [Button::Left, Button::Right].into_iter().enumerate() {
            if self.held[i] {
                let _ = self.enigo.button(button, Direction::Release);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_headset_position_maps_to_the_whole_screen_and_never_outside() {
        assert_eq!(to_pixels(0.0, 0.0, 3440, 1440), (0, 0));
        assert_eq!(to_pixels(1.0, 1.0, 3440, 1440), (3439, 1439));
        assert_eq!(to_pixels(0.5, 0.5, 3440, 1440), (1720, 720));
        assert_eq!(to_pixels(-0.3, 1.7, 1920, 1080), (0, 1079), "outside the image it stays on the edge");
    }

    #[test]
    fn keys_by_name_and_single_characters() {
        assert_eq!(key_for("enter"), Some(Key::Return));
        assert_eq!(key_for("backspace"), Some(Key::Backspace));
        assert_eq!(key_for("left"), Some(Key::LeftArrow));
        assert_eq!(key_for("s"), Some(Key::Unicode('s')));
        assert_eq!(key_for("ñ"), Some(Key::Unicode('ñ')));
        assert_eq!(key_for("nope"), None);
        assert_eq!(key_for(""), None);
    }
}
