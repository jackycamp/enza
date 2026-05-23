use crate::{
    layout::{RowContext, RowKind},
    diff::DiffFile,
    note::NoteTarget,
};

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
