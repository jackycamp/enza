#[derive(Debug)]
pub struct DiffViewState {
    pub selected_file: usize,
    pub selected_hunk: usize,
    pub cursor_row: usize,
    pub selection_anchor: Option<usize>,
    pub scroll: u16,
}
