//! CPU rendering of the Atlas Code panels. It does not depend on XR, so it is
//! tested and previewed on the PC (see `examples/preview.rs`).

mod alert;
mod canvas;
mod cards;
mod keyboard;
mod panel;

pub use alert::{AlertPanel, AlertView, ALERT_H, ALERT_W, ALERT_W_M};
pub use canvas::{Canvas, Rgb};
pub use cards::{CardsPanel, CardsView, CARD_COLS, CARD_ROWS, MAX_CARDS};
pub use keyboard::{KeyView, KeyboardPanel, KeyboardView, PadView, KB_H, KB_PX_PER_M, KB_W, KB_W_M};
pub use panel::{CodePanel, Theme};
