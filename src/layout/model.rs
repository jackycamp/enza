use ratatui::text::Line;

#[derive(Clone, Debug)]
pub struct HunkRange {
    pub file_index: usize,
    pub hunk_index: usize,
    pub start: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RowContext {
    pub file_index: Option<usize>,
    pub hunk_index: Option<usize>,
    pub kind: RowKind,
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
    pub note_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RowKind {
    #[default]
    Separator,
    FileHeader,
    HunkHeader,
    DiffLine,
    Note,
    Spacer,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub inline_width: usize,
    pub side_by_side_width: usize,
    pub target_generation: u64,
    pub target_file: usize,
    pub target_hunk: usize,
    pub base: BaseLayout,
    pub inline_rows: Vec<RenderRow>,
    pub side_by_side_rows: Vec<RenderRow>,
    pub hunk_ranges: Vec<HunkRange>,
    pub row_contexts: Vec<RowContext>,
}

#[derive(Clone, Debug)]
pub struct BaseLayout {
    #[allow(dead_code)]
    pub tree: LayoutTree,
    pub inline_rows: Vec<RenderRow>,
    pub side_by_side_rows: Vec<RenderRow>,
    pub hunk_ranges: Vec<HunkRange>,
    pub row_contexts: Vec<RowContext>,
}

#[derive(Clone, Debug)]
pub struct LayoutTree {
    pub files: Vec<FileNode>,
}

#[derive(Clone, Debug)]
pub struct FileNode {
    #[allow(dead_code)]
    pub file_index: usize,
    #[allow(dead_code)]
    pub status: NodeStatus,
    pub header: CachedRows,
    pub hunks: Vec<HunkNode>,
    pub trailing_spacer: CachedRows,
}

#[derive(Clone, Debug)]
pub struct HunkNode {
    pub file_index: usize,
    pub hunk_index: usize,
    pub status: NodeStatus,
    pub rows: CachedRows,
}

#[derive(Clone, Debug, Default)]
pub struct CachedRows {
    pub inline_rows: Vec<RenderRow>,
    pub side_by_side_rows: Vec<RenderRow>,
    pub row_contexts: Vec<RowContext>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeStatus {
    Ready,
    Loading,
    #[allow(dead_code)]
    Unbuilt,
    #[allow(dead_code)]
    Dirty,
}

#[derive(Clone, Copy, Debug)]
pub struct RowViewState {
    pub scroll: u16,
    pub selected_file: usize,
    pub selected_hunk: usize,
    pub cursor_row: usize,
    pub cursor_focused: bool,
    pub selected_rows: Option<(usize, usize)>,
}

#[derive(Clone, Debug)]
pub enum RenderRow {
    Static(Line<'static>),
    FileHeader {
        file_index: usize,
        normal: Line<'static>,
        selected: Line<'static>,
    },
    HunkHeader {
        file_index: usize,
        hunk_index: usize,
        normal: Line<'static>,
        selected: Line<'static>,
    },
    Note(Line<'static>),
}
