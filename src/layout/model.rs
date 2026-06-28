use ratatui::text::Line;

#[derive(Clone, Debug)]
pub struct HunkRange {
    pub file_index: usize,
    pub hunk_index: usize,
    pub start: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RowId {
    FileSeparator {
        file_index: usize,
    },
    FileHeader {
        file_index: usize,
    },
    HunkHeader {
        file_index: usize,
        hunk_index: usize,
    },
    DiffLine {
        file_index: usize,
        hunk_index: usize,
        line_index: usize,
    },
    HunkSpacer {
        file_index: usize,
        hunk_index: usize,
    },
    FileSpacer {
        file_index: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlannedHunk {
    pub file_index: usize,
    pub hunk_index: usize,
    /// Base row index of the hunk header.
    pub start: usize,
    /// Number of diff lines; the header and trailing spacer are derived.
    pub line_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedFile {
    pub file_index: usize,
    /// Base row index of the file separator.
    pub start: usize,
    /// Base row index of the blank row after the file's hunks.
    pub trailing_spacer: usize,
    pub hunks: Vec<PlannedHunk>,
}

#[derive(Clone, Debug)]
pub struct LayoutPlan {
    /// File and hunk start rows. This replaces storing one planned row per line.
    pub files: Vec<PlannedFile>,
    pub hunk_ranges: Vec<HunkRange>,
    /// Total base rows before note rows are inserted.
    pub row_count: usize,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub inline_width: usize,
    pub side_by_side_width: usize,
    pub target_generation: u64,
    pub target_file: usize,
    pub target_hunk: usize,
    pub base: BaseLayout,
    pub hunk_ranges: Vec<HunkRange>,
    pub note_insertions: Vec<NoteInsertion>,
    pub row_count: usize,
}

#[derive(Clone, Debug)]
pub struct BaseLayout {
    #[allow(dead_code)]
    pub tree: LayoutTree,
    pub plan: LayoutPlan,
    pub hunk_ranges: Vec<HunkRange>,
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
    /// Rendered rows for loaded hunks and always-loaded file header/separator rows.
    pub inline_rows: Vec<RenderRow>,
    pub side_by_side_rows: Vec<RenderRow>,
    pub row_contexts: Vec<RowContext>,
}

#[derive(Clone, Debug)]
pub struct NoteInsertion {
    /// Base row before which these note rows are inserted.
    pub base_index: usize,
    pub inline_rows: Vec<RenderRow>,
    pub side_by_side_rows: Vec<RenderRow>,
    pub context: RowContext,
}

impl NoteInsertion {
    /// Returns how many rendered rows this note inserts before its base row.
    pub fn len(&self) -> usize {
        self.inline_rows.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutRowLocation {
    Base {
        base_index: usize,
    },
    Note {
        insertion_index: usize,
        row_offset: usize,
    },
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
