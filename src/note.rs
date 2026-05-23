use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    cache::{RowContext, RowKind},
    diff::DiffFile,
};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Note {
    pub id: u64,
    pub target: NoteTarget,
    pub body: String,
    pub created_at_ms: u128,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum NoteTarget {
    File {
        file_path: String,
    },
    Hunk {
        file_path: String,
        hunk_header: String,
    },
    Line {
        file_path: String,
        old_lineno: Option<usize>,
        new_lineno: Option<usize>,
    },
    Range {
        file_path: String,
        start_old_lineno: Option<usize>,
        start_new_lineno: Option<usize>,
        end_old_lineno: Option<usize>,
        end_new_lineno: Option<usize>,
    },
}

impl Note {
    pub fn new(id: u64, target: NoteTarget, body: String) -> Self {
        Self {
            id,
            target,
            body,
            created_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0),
        }
    }
}

pub fn note_target_for_row(
    files: &[DiffFile],
    row_contexts: &[RowContext],
    row: usize,
) -> Option<NoteTarget> {
    let context = row_contexts.get(row)?;
    let file_index = context.file_index?;
    let file = files.get(file_index)?;
    let file_path = file.path.clone();

    match context.kind {
        RowKind::FileHeader | RowKind::Separator | RowKind::Spacer | RowKind::Note => {
            Some(NoteTarget::File { file_path })
        }
        RowKind::HunkHeader => {
            let hunk_index = context.hunk_index?;
            let hunk = file.hunks.get(hunk_index)?;
            Some(NoteTarget::Hunk {
                file_path,
                hunk_header: hunk.header.clone(),
            })
        }
        RowKind::DiffLine => Some(NoteTarget::Line {
            file_path,
            old_lineno: context.old_lineno,
            new_lineno: context.new_lineno,
        }),
    }
}

pub fn note_target_for_range(
    files: &[DiffFile],
    row_contexts: &[RowContext],
    start: usize,
    end: usize,
    fallback_row: usize,
) -> Option<NoteTarget> {
    let start_context = row_contexts.get(start)?;
    let end_context = row_contexts.get(end)?;
    let file_index = start_context.file_index?;
    if end_context.file_index != Some(file_index) {
        return note_target_for_row(files, row_contexts, fallback_row);
    }

    let file_path = files.get(file_index)?.path.clone();
    Some(NoteTarget::Range {
        file_path,
        start_old_lineno: start_context.old_lineno,
        start_new_lineno: start_context.new_lineno,
        end_old_lineno: end_context.old_lineno,
        end_new_lineno: end_context.new_lineno,
    })
}
