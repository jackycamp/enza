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
    /// The rendered row within a note card. Zero is the top border.
    pub note_row_offset: usize,
}

impl RowContext {
    pub fn separator(file_index: usize) -> Self {
        Self {
            file_index: Some(file_index),
            hunk_index: None,
            kind: RowKind::Separator,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
            note_row_offset: 0,
        }
    }

    pub fn file_header(file_index: usize) -> Self {
        Self {
            file_index: Some(file_index),
            hunk_index: None,
            kind: RowKind::FileHeader,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
            note_row_offset: 0,
        }
    }

    pub fn hunk_header(file_index: usize, hunk_index: usize) -> Self {
        Self {
            file_index: Some(file_index),
            hunk_index: Some(hunk_index),
            kind: RowKind::HunkHeader,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
            note_row_offset: 0,
        }
    }

    pub fn diff_line(
        file_index: usize,
        hunk_index: usize,
        old_lineno: Option<usize>,
        new_lineno: Option<usize>,
    ) -> Self {
        Self {
            file_index: Some(file_index),
            hunk_index: Some(hunk_index),
            kind: RowKind::DiffLine,
            old_lineno,
            new_lineno,
            note_id: None,
            note_row_offset: 0,
        }
    }

    pub fn spacer(file_index: usize, hunk_index: usize) -> Self {
        Self {
            file_index: Some(file_index),
            hunk_index: Some(hunk_index),
            kind: RowKind::Spacer,
            old_lineno: None,
            new_lineno: None,
            note_id: None,
            note_row_offset: 0,
        }
    }

    pub fn note(anchor: Self, note_id: u64) -> Self {
        Self {
            file_index: anchor.file_index,
            hunk_index: anchor.hunk_index,
            kind: RowKind::Note,
            old_lineno: None,
            new_lineno: None,
            note_id: Some(note_id),
            note_row_offset: 0,
        }
    }

    pub fn with_note_row_offset(mut self, note_row_offset: usize) -> Self {
        self.note_row_offset = note_row_offset;
        self
    }
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
    /// One past the last base row belonging to this file.
    pub end: usize,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LayoutWidths {
    pub inline: usize,
    pub side_by_side: usize,
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

impl RenderRow {
    pub(crate) fn static_line(line: Line<'static>) -> Self {
        Self::Static(line)
    }

    pub(crate) fn note(line: Line<'static>) -> Self {
        Self::Note(line)
    }

    pub(crate) fn file_header(
        file_index: usize,
        normal: Line<'static>,
        selected: Line<'static>,
    ) -> Self {
        Self::FileHeader {
            file_index,
            normal,
            selected,
        }
    }

    pub(crate) fn hunk_header(
        file_index: usize,
        hunk_index: usize,
        normal: Line<'static>,
        selected: Line<'static>,
    ) -> Self {
        Self::HunkHeader {
            file_index,
            hunk_index,
            normal,
            selected,
        }
    }

    pub(crate) fn into_line(self, view: &RowViewState) -> Line<'static> {
        match self {
            Self::Static(line) | Self::Note(line) => line,
            Self::FileHeader {
                file_index,
                normal,
                selected,
            } => {
                if file_index == view.selected_file {
                    selected
                } else {
                    normal
                }
            }
            Self::HunkHeader {
                file_index,
                hunk_index,
                normal,
                selected,
            } => {
                if file_index == view.selected_file && hunk_index == view.selected_hunk {
                    selected
                } else {
                    normal
                }
            }
        }
    }
}
