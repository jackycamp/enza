//! Cursor, selection and scrolling state for the main diff pane.

use crate::tui::layout::{FileDisplayState, RowContext};

#[derive(Debug)]
pub struct MainPaneState {
    pub file_states: Vec<FileDisplayState>,
    pub selected_file: usize,
    pub selected_hunk: usize,
    pub cursor_row: usize,
    pub selection_anchor: Option<RowContext>,
    pub scroll: u16,
}
