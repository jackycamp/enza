use crate::diff::DiffSession;
use crate::layout::base_layout::BaseLayout;
use crate::layout::note::{
    build_note_rows, note_wrap_width, render_note_rows, render_side_by_side_note_rows,
};
use crate::layout::plan::plan_row_contexts;
use crate::layout::primitives::{
    HunkRange, LayoutWidths, NoteInsertion, RenderRow, RowContext, RowKind,
};
use crate::note::{Note, NoteId, NoteTarget};

pub(super) struct NoteOverlay {
    pub insertions: Vec<NoteInsertion>,
    pub hunk_ranges: Vec<HunkRange>,
    pub row_count: usize,
}

#[derive(Clone, Copy)]
pub(super) struct NoteOverlayRequest<'a> {
    pub session: &'a DiffSession,
    pub base: &'a BaseLayout,
    pub notes: &'a [Note],
    pub expanded_note_ids: &'a [NoteId],
    pub reply_composer_note_id: Option<NoteId>,
    pub widths: LayoutWidths,
}

impl NoteOverlay {
    pub(super) fn new(request: NoteOverlayRequest<'_>) -> Self {
        let overlay = NoteInsertions::new(request);

        Self {
            hunk_ranges: adjust_hunk_ranges_for_insertions(
                request.base.hunk_ranges.clone(),
                &overlay.inserted_before_or_at_base,
            ),
            row_count: request.base.plan.row_count + overlay.inserted_total,
            insertions: overlay.insertions,
        }
    }
}

pub fn build_note_anchors<'a>(
    session: &DiffSession,
    notes: &'a [Note],
    row_contexts: &[RowContext],
) -> Vec<(usize, &'a Note)> {
    notes
        .iter()
        .filter_map(|note| note_anchor_row(session, row_contexts, note).map(|row| (row, note)))
        .collect()
}

fn note_anchor_row(
    session: &DiffSession,
    row_contexts: &[RowContext],
    note: &Note,
) -> Option<usize> {
    match &note.target {
        NoteTarget::File { file_path } => row_contexts.iter().position(|context| {
            matches!(context.kind, RowKind::FileHeader)
                && context
                    .file_index
                    .and_then(|index| session.files.get(index))
                    .is_some_and(|file| &file.path == file_path)
        }),

        NoteTarget::Hunk {
            file_path,
            hunk_header,
        } => row_contexts.iter().position(|context| {
            matches!(context.kind, RowKind::DiffLine | RowKind::HunkHeader)
                && context
                    .file_index
                    .and_then(|index| session.files.get(index))
                    .is_some_and(|file| {
                        &file.path == file_path
                            && context
                                .hunk_index
                                .and_then(|hunk_index| file.hunks.get(hunk_index))
                                .is_some_and(|hunk| &hunk.header == hunk_header)
                    })
        }),

        NoteTarget::Line {
            file_path,
            old_lineno,
            new_lineno,
        } => row_contexts.iter().position(|context| {
            DiffLineAnchor::new(file_path, old_lineno, new_lineno).matches(session, context)
        }),

        NoteTarget::Range {
            file_path,
            start_old_lineno,
            start_new_lineno,
            end_old_lineno,
            end_new_lineno,
        } => {
            let start_anchor = DiffLineAnchor::new(file_path, start_old_lineno, start_new_lineno);
            let end_anchor = DiffLineAnchor::new(file_path, end_old_lineno, end_new_lineno);
            let start = row_contexts
                .iter()
                .position(|context| start_anchor.matches(session, context))?;
            row_contexts
                .iter()
                .any(|context| end_anchor.matches(session, context))
                .then_some(start)
        }
    }
}

#[derive(Clone, Copy)]
struct DiffLineAnchor<'a> {
    file_path: &'a str,
    old_lineno: Option<usize>,
    new_lineno: Option<usize>,
}

impl<'a> DiffLineAnchor<'a> {
    fn new(file_path: &'a str, old_lineno: &Option<usize>, new_lineno: &Option<usize>) -> Self {
        Self {
            file_path,
            old_lineno: *old_lineno,
            new_lineno: *new_lineno,
        }
    }

    fn matches(self, session: &DiffSession, context: &RowContext) -> bool {
        matches!(context.kind, RowKind::DiffLine)
            && context.old_lineno == self.old_lineno
            && context.new_lineno == self.new_lineno
            && context
                .file_index
                .and_then(|index| session.files.get(index))
                .is_some_and(|file| file.path == self.file_path)
    }
}

struct NoteInsertions {
    insertions: Vec<NoteInsertion>,
    inserted_before_or_at_base: Vec<usize>,
    inserted_total: usize,
}

impl NoteInsertions {
    fn new(request: NoteOverlayRequest<'_>) -> Self {
        if request.notes.is_empty() {
            return Self {
                insertions: Vec::new(),
                inserted_before_or_at_base: vec![0usize; request.base.plan.row_count + 1],
                inserted_total: 0,
            };
        }

        let base_row_contexts = plan_row_contexts(request.session, &request.base.plan);
        let note_anchors = build_note_anchors(request.session, request.notes, &base_row_contexts);
        let mut insertions = Vec::new();
        let mut inserted_before_or_at_base = vec![0usize; request.base.plan.row_count + 1];
        let mut inserted_total = 0usize;
        for base_index in 0..request.base.plan.row_count {
            for note in note_anchors
                .iter()
                .filter(|(anchor_index, _)| *anchor_index == base_index)
                .map(|(_, note)| note)
            {
                let expanded = request.expanded_note_ids.contains(&note.id);
                let composer_active = request.reply_composer_note_id == Some(note.id);
                let note_rows = build_note_rows(
                    note,
                    note_wrap_width(note, request.widths.inline, request.widths.side_by_side),
                    expanded,
                    composer_active,
                );
                let context = RowContext::note(base_row_contexts[base_index], note.id);
                let insertion = build_note_insertion(NoteInsertionRequest {
                    base_index,
                    note_rows: &note_rows,
                    note,
                    context,
                    widths: request.widths,
                });
                inserted_total += insertion.len();
                insertions.push(insertion);
            }
            inserted_before_or_at_base[base_index] = inserted_total;
        }
        inserted_before_or_at_base[request.base.plan.row_count] = inserted_total;

        Self {
            insertions,
            inserted_before_or_at_base,
            inserted_total,
        }
    }
}

struct NoteInsertionRequest<'a> {
    base_index: usize,
    note_rows: &'a [crate::layout::note::NoteRow],
    note: &'a Note,
    context: RowContext,
    widths: LayoutWidths,
}

fn build_note_insertion(request: NoteInsertionRequest<'_>) -> NoteInsertion {
    let inline = render_note_rows(request.note, request.note_rows, request.widths.inline);
    let side_by_side =
        render_side_by_side_note_rows(request.note, request.note_rows, request.widths.side_by_side);
    debug_assert_eq!(inline.composer_bounds, side_by_side.composer_bounds);

    NoteInsertion {
        base_index: request.base_index,
        inline_rows: inline.rows.into_iter().map(RenderRow::note).collect(),
        side_by_side_rows: side_by_side.rows.into_iter().map(RenderRow::note).collect(),
        context: request.context,
        composer_bounds: inline.composer_bounds,
    }
}

fn adjust_hunk_ranges_for_insertions(
    hunk_ranges: Vec<HunkRange>,
    inserted_before_or_at_base: &[usize],
) -> Vec<HunkRange> {
    hunk_ranges
        .into_iter()
        .map(|range| HunkRange {
            file_index: range.file_index,
            hunk_index: range.hunk_index,
            start: range.start + inserted_before_or_at_base[range.start],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffFile, DiffHunk, DiffLine};
    use crate::layout::base_layout::BaseLayout;
    use crate::layout::layout_tree::LayoutTree;
    use crate::layout::primitives::{LayoutPlan, LayoutWidths};

    #[test]
    fn note_anchors_resolve_file_hunk_line_and_range_targets() {
        let session = session_with_one_hunk();
        let plan = LayoutPlan::new(&session);
        let contexts = plan_row_contexts(&session, &plan);
        let notes = vec![
            Note::new(
                1,
                NoteTarget::File {
                    file_path: "test.rs".to_string(),
                },
                "file".to_string(),
            ),
            Note::new(
                2,
                NoteTarget::Hunk {
                    file_path: "test.rs".to_string(),
                    hunk_header: "@@ hunk 0 @@".to_string(),
                },
                "hunk".to_string(),
            ),
            Note::new(
                3,
                NoteTarget::Line {
                    file_path: "test.rs".to_string(),
                    old_lineno: None,
                    new_lineno: Some(2),
                },
                "line".to_string(),
            ),
            Note::new(
                4,
                NoteTarget::Range {
                    file_path: "test.rs".to_string(),
                    start_old_lineno: Some(1),
                    start_new_lineno: Some(1),
                    end_old_lineno: None,
                    end_new_lineno: Some(2),
                },
                "range".to_string(),
            ),
        ];

        let anchors: Vec<_> = build_note_anchors(&session, &notes, &contexts)
            .into_iter()
            .map(|(row, note)| (row, note.id))
            .collect();

        assert_eq!(anchors, vec![(1, 1), (2, 2), (4, 3), (3, 4)]);
    }

    #[test]
    fn note_overlay_exposes_reply_composer_bounds() {
        let session = session_with_one_hunk();
        let base = base_layout(&session);
        let note = Note::new_agent(
            1,
            NoteTarget::File {
                file_path: "test.rs".to_string(),
            },
            crate::note::AgentProvider::Codex,
            "Explain this".to_string(),
            1,
        );

        let overlay = NoteOverlay::new(NoteOverlayRequest {
            session: &session,
            base: &base,
            notes: &[note],
            expanded_note_ids: &[1],
            reply_composer_note_id: Some(1),
            widths: widths(),
        });

        let bounds = overlay.insertions[0].composer_bounds.clone().unwrap();
        assert_eq!(bounds.len(), crate::layout::note::NOTE_COMPOSER_HEIGHT);
        assert!(bounds.end < overlay.insertions[0].len());
    }

    #[test]
    fn note_overlay_adjusts_hunk_ranges_by_full_inserted_height() {
        let session = session_with_two_hunks();
        let base = base_layout(&session);
        let note = Note::new(
            1,
            NoteTarget::File {
                file_path: "test.rs".to_string(),
            },
            "file note".to_string(),
        );

        let overlay = NoteOverlay::new(NoteOverlayRequest {
            session: &session,
            base: &base,
            notes: &[note],
            expanded_note_ids: &[],
            reply_composer_note_id: None,
            widths: widths(),
        });

        assert_eq!(overlay.insertions[0].len(), 3);
        assert_eq!(overlay.row_count, base.plan.row_count + 3);
        assert_eq!(overlay.hunk_ranges[0].start, base.hunk_ranges[0].start + 3);
        assert_eq!(overlay.hunk_ranges[1].start, base.hunk_ranges[1].start + 3);
    }

    fn base_layout(session: &DiffSession) -> BaseLayout {
        let plan = LayoutPlan::new(session);
        BaseLayout {
            tree: LayoutTree::new(session, widths()),
            hunk_ranges: plan.hunk_ranges.clone(),
            plan,
        }
    }

    fn widths() -> LayoutWidths {
        LayoutWidths {
            inline: 30,
            side_by_side: 30,
        }
    }

    fn session_with_one_hunk() -> DiffSession {
        DiffSession {
            files: vec![DiffFile {
                path: "test.rs".to_string(),
                old_path: "test.rs".to_string(),
                new_path: "test.rs".to_string(),
                hunks: vec![DiffHunk {
                    header: "@@ hunk 0 @@".to_string(),
                    lines: vec![
                        DiffLine::Context {
                            old_lineno: 1,
                            new_lineno: 1,
                            text: "same".to_string(),
                        },
                        DiffLine::Added {
                            new_lineno: 2,
                            text: "added".to_string(),
                        },
                    ],
                }],
            }],
        }
    }

    fn session_with_two_hunks() -> DiffSession {
        let mut session = session_with_one_hunk();
        session.files[0].hunks.push(DiffHunk {
            header: "@@ hunk 1 @@".to_string(),
            lines: vec![DiffLine::Context {
                old_lineno: 3,
                new_lineno: 3,
                text: "later".to_string(),
            }],
        });
        session
    }
}
