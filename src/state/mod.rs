mod diff_view;
mod global;
mod notes;
mod root;
mod sidebar;

pub use diff_view::DiffViewState;
pub use global::{DiffMode, FocusPane, GlobalState};
pub use notes::{NoteInputResult, NoteState};
pub use root::App;
pub use sidebar::{SidebarEntry, SidebarEntryKind, SidebarState};
