//! Editing core of Atlas Code. It does not depend on XR or Android,
//! so it is tested on the host with `cargo test`.

mod browser;
mod buffer;
mod editor;
mod file_info;
mod log;
mod mode;
mod workspace;

pub use browser::{parent_of, Browser, Entry};
pub use buffer::{Buffer, Cursor, Snapshot};
pub use editor::{Action, Editor, GitRequest, Key};
pub use file_info::{ago, human_size, FileInfo};
pub use log::{Log, Progress, Secret};
pub use mode::{Mode, Motion};
pub use workspace::{perform, Workspace};
