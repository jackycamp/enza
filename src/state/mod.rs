//! Mutable state for an active diff review.
//!
//! [`App`] owns transitions that affect more than one interface area. Local state
//! types own local rules. Cross-area transitions keep selection, sidebar
//! position, layout rows, note anchors and agent runs consistent. State code
//! does not read terminal events or draw the interface.

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
