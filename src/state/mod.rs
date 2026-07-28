mod agent;
mod global;
mod main_pane;
mod notes;
mod root;
mod selection;
mod sidebar;

pub use global::{DiffMode, FocusPane, GlobalState};
pub use main_pane::MainPaneState;
pub use notes::{NoteComposerMode, NoteInputResult, NoteState};
pub use root::App;
pub use selection::{note_target_for_range, note_target_for_row};
pub use sidebar::{SidebarEntry, SidebarEntryKind, SidebarState};
