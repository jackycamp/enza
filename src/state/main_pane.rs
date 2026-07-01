use crate::layout::RowContext;

#[derive(Debug)]
pub struct MainPaneState {
    pub selected_file: usize,
    pub selected_hunk: usize,
    pub cursor_row: usize,
    pub selection_anchor: Option<RowContext>,
    pub scroll: u16,
}
