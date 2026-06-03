use crate::layout::RowContext;

#[derive(Debug)]
pub struct MainPaneState {
    pub selected_file: usize,
    pub selected_hunk: usize,
    pub cursor_row: usize,
    pub cursor_target: Option<RowContext>,
    pub selection_anchor: Option<usize>,
    pub scroll: u16,
}
