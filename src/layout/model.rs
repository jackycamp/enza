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
    pub inline_rows: Vec<RenderRow>,
    pub side_by_side_rows: Vec<RenderRow>,
    pub hunk_ranges: Vec<HunkRange>,
    pub row_contexts: Vec<RowContext>,
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
