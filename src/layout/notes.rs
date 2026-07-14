use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::diff::DiffSession;
use crate::layout::base_layout::BaseLayout;
use crate::layout::lines::{combined_side_line, split_side_by_side_width};
use crate::layout::plan::plan_row_contexts;
use crate::layout::primitives::{
    HunkRange, LayoutWidths, NoteInsertion, RenderRow, RowContext, RowKind,
};
use crate::layout::text::{fit_text, truncate_with_ellipsis, wrap_text};
use crate::mention_style::{styled_mentions, styled_note_text};
use crate::note::{AGENT_ACTIVITY_INTERVAL, AgentStatus, MessageAuthor, Note, NoteTarget};

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
    pub expanded_note_ids: &'a [u64],
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NoteSide {
    Full,
    Left,
    Right,
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

pub fn build_note_rows(note: &Note, width: usize, expanded: bool) -> Vec<String> {
    let body_width = width.saturating_sub(4).max(8);
    if note.is_agent_thread() {
        return build_agent_note_rows(note, body_width, expanded);
    }

    let mut rows = wrap_note_body(note.personal_body().unwrap_or_default(), body_width);
    if rows.is_empty() {
        rows.push(String::new());
    }

    if !expanded && rows.len() > 2 {
        rows.truncate(2);
        if let Some(last) = rows.last_mut() {
            *last = truncate_with_ellipsis(last, body_width);
        }
    }

    rows
}

fn build_agent_note_rows(note: &Note, width: usize, expanded: bool) -> Vec<String> {
    if !expanded {
        return build_collapsed_agent_rows(note, width);
    }

    let mut rows = Vec::new();
    for message in &note.messages {
        if !rows.is_empty() {
            rows.push(String::new());
        }
        let author = match message.author {
            MessageAuthor::User => "You:",
            MessageAuthor::Agent(provider) => match provider {
                crate::note::AgentProvider::Codex => "Codex:",
                crate::note::AgentProvider::Claude => "Claude:",
            },
        };
        rows.push(author.to_string());
        rows.extend(wrap_note_body(&message.body, width));
    }

    append_agent_status_rows(note, width, &mut rows);
    rows.push(agent_action_hint(note, true));
    if note.agent.as_ref().is_some_and(|agent| agent.composer_open) {
        rows.extend([String::new(), String::new(), String::new()]);
    }
    rows
}

fn build_collapsed_agent_rows(note: &Note, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    if let Some(agent) = &note.agent
        && !matches!(agent.status, AgentStatus::Complete)
    {
        if let Some(latest_user) = note
            .messages
            .iter()
            .rev()
            .find(|message| message.author == MessageAuthor::User)
        {
            rows.push(message_summary("You", &latest_user.body, width));
        }
        rows.push(agent_status_summary(&agent.status));
        if note.messages.len() > 1 {
            rows.push(format!("{} earlier messages", note.messages.len() - 1));
        }
        rows.push(agent_action_hint(note, false));
        return rows;
    }

    if let Some(first_user) = note
        .messages
        .iter()
        .find(|message| message.author == MessageAuthor::User)
    {
        rows.push(message_summary("You", &first_user.body, width));
    }

    let latest_agent = note.messages.iter().rev().find_map(|message| {
        let MessageAuthor::Agent(provider) = message.author else {
            return None;
        };
        Some((provider, message))
    });
    if let Some((provider, message)) = latest_agent {
        rows.push(message_summary(provider.label(), &message.body, width));
    } else if let Some(agent) = &note.agent {
        rows.push(agent_status_summary(&agent.status));
    }

    if note.messages.len() > 2 {
        rows.push(format!("{} more messages", note.messages.len() - 2));
    }

    rows.push(agent_action_hint(note, false));

    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

fn append_agent_status_rows(note: &Note, width: usize, rows: &mut Vec<String>) {
    let Some(agent) = &note.agent else {
        return;
    };
    match &agent.status {
        AgentStatus::Queued | AgentStatus::Running { .. } => {
            rows.push(String::new());
            rows.push(agent_status_summary(&agent.status));
        }
        AgentStatus::Failed(failure) => {
            rows.push(String::new());
            rows.push("Enza".to_string());
            rows.extend(wrap_note_body(&failure.message, width));
        }
        AgentStatus::Cancelled => {
            rows.push(String::new());
            rows.push("Enza".to_string());
            rows.push("The agent request was cancelled.".to_string());
        }
        AgentStatus::Complete => {}
    }
}

fn agent_status_summary(status: &AgentStatus) -> String {
    match status {
        AgentStatus::Queued => "Waiting to start…".to_string(),
        AgentStatus::Running {
            started_at,
            slow: true,
        } => format!(
            "{} Still responding · {}m",
            agent_activity_glyph(started_at.elapsed()),
            started_at.elapsed().as_secs().div_ceil(60)
        ),
        AgentStatus::Running { started_at, .. } => {
            format!("{} Responding…", agent_activity_glyph(started_at.elapsed()))
        }
        AgentStatus::Complete => "Answered".to_string(),
        AgentStatus::Failed(failure) if failure.retryable => {
            "Agent request failed · r to retry".to_string()
        }
        AgentStatus::Failed(_) => "Agent request failed".to_string(),
        AgentStatus::Cancelled => "Agent request cancelled · r to retry".to_string(),
    }
}

fn agent_action_hint(note: &Note, expanded: bool) -> String {
    if note.agent.as_ref().is_some_and(|agent| agent.composer_open) {
        return "Enter to send · Esc to cancel".to_string();
    }
    let toggle = if expanded {
        "Enter to collapse"
    } else {
        "Enter to expand"
    };
    let Some(agent) = &note.agent else {
        return toggle.to_string();
    };
    let action = match &agent.status {
        AgentStatus::Queued | AgentStatus::Running { .. } => Some("c to cancel"),
        AgentStatus::Complete if agent.session_id.is_some() => Some("n to reply"),
        AgentStatus::Failed(failure) if failure.retryable => Some("r to retry"),
        AgentStatus::Cancelled => Some("r to retry"),
        _ => None,
    };
    action.map_or_else(
        || toggle.to_string(),
        |action| format!("{toggle} · {action}"),
    )
}

fn agent_activity_glyph(elapsed: std::time::Duration) -> &'static str {
    let phase = elapsed.as_millis() / AGENT_ACTIVITY_INTERVAL.as_millis();
    if phase.is_multiple_of(2) { "+" } else { "−" }
}

fn message_summary(author: &str, body: &str, width: usize) -> String {
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_with_ellipsis(&format!("{author}: {body}"), width)
}

fn wrap_note_body(body: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            rows.push(String::new());
            continue;
        }
        rows.extend(wrap_text(line, width));
    }
    rows
}

pub fn render_note_rows(note: &Note, rows: &[String], width: usize) -> Vec<Line<'static>> {
    let inner_width = width.saturating_sub(2).max(4);
    let border_style = Style::default().fg(Color::DarkGray);
    let content_style = Style::default().fg(Color::White);
    let mut rendered = Vec::with_capacity(rows.len() + 2);

    let title = note.agent.as_ref().map(|agent| {
        format!(
            "{} · {}",
            agent.provider.mention(),
            match &agent.status {
                AgentStatus::Queued => "queued",
                AgentStatus::Running { slow: false, .. } => "responding",
                AgentStatus::Running { slow: true, .. } => "still responding",
                AgentStatus::Complete => "answered",
                AgentStatus::Failed(failure)
                    if failure.kind == crate::note::AgentFailureKind::Timeout =>
                    "timed out",
                AgentStatus::Failed(_) => "failed",
                AgentStatus::Cancelled => "cancelled",
            }
        )
    });
    let top = match title {
        Some(title) => {
            let title = truncate_with_ellipsis(&format!("─ {title} "), inner_width);
            format!(
                "{title}{}",
                "─".repeat(inner_width.saturating_sub(title.chars().count()))
            )
        }
        None => "─".repeat(inner_width),
    };
    let mut top_spans = vec![Span::styled("┌".to_string(), border_style)];
    top_spans.extend(styled_mentions(&top, border_style));
    top_spans.push(Span::styled("┐".to_string(), border_style));
    rendered.push(Line::from(top_spans));

    for row in rows {
        let content = fit_text(&format!(" {row}"), inner_width);
        let mut spans = vec![Span::styled("│".to_string(), border_style)];
        spans.extend(styled_note_text(&content, content_style));
        spans.push(Span::styled("│".to_string(), border_style));
        rendered.push(Line::from(spans));
    }

    rendered.push(Line::from(vec![
        Span::styled("└".to_string(), border_style),
        Span::styled("─".repeat(inner_width), border_style),
        Span::styled("┘".to_string(), border_style),
    ]));

    rendered
}

pub fn render_side_by_side_note_rows(
    rows: &[String],
    width: usize,
    note: &Note,
) -> Vec<Line<'static>> {
    let side = note_side_impl(note);
    if side == NoteSide::Full {
        return render_note_rows(note, rows, width);
    }

    let (left_width, right_width) = split_side_by_side_width(width);
    let note_width = match side {
        NoteSide::Left => left_width,
        NoteSide::Right => right_width,
        NoteSide::Full => width,
    };
    let note_rows = render_note_rows(note, rows, note_width);
    let divider_style = Style::default().fg(Color::DarkGray);

    note_rows
        .into_iter()
        .map(|note_row| match side {
            NoteSide::Left => combined_side_line(note_row, blank_note_side_line(right_width)),
            NoteSide::Right => {
                let mut spans = blank_note_side_line(left_width).spans;
                spans.push(Span::styled(" │ ".to_string(), divider_style));
                spans.extend(note_row.spans);
                Line::from(spans)
            }
            NoteSide::Full => unreachable!(),
        })
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

fn blank_note_side_line(width: usize) -> Line<'static> {
    Line::from(Span::raw(" ".repeat(width)))
}

fn note_side_impl(note: &Note) -> NoteSide {
    match &note.target {
        NoteTarget::Line {
            old_lineno: Some(_),
            new_lineno: None,
            ..
        }
        | NoteTarget::Range {
            start_old_lineno: Some(_),
            start_new_lineno: None,
            ..
        } => NoteSide::Left,
        NoteTarget::Line {
            old_lineno: None,
            new_lineno: Some(_),
            ..
        }
        | NoteTarget::Range {
            start_old_lineno: None,
            start_new_lineno: Some(_),
            ..
        } => NoteSide::Right,
        _ => NoteSide::Full,
    }
}

fn note_wrap_width(note: &Note, widths: LayoutWidths) -> usize {
    let (left_width, right_width) = split_side_by_side_width(widths.side_by_side);
    let side_by_side_note_width = match note_side_impl(note) {
        NoteSide::Full => widths.side_by_side,
        NoteSide::Left => left_width,
        NoteSide::Right => right_width,
    };
    widths.inline.min(side_by_side_note_width)
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
                let note_rows =
                    build_note_rows(note, note_wrap_width(note, request.widths), expanded);
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
    note_rows: &'a [String],
    note: &'a Note,
    context: RowContext,
    widths: LayoutWidths,
}

fn build_note_insertion(request: NoteInsertionRequest<'_>) -> NoteInsertion {
    NoteInsertion {
        base_index: request.base_index,
        inline_rows: render_note_rows(request.note, request.note_rows, request.widths.inline)
            .into_iter()
            .map(RenderRow::note)
            .collect(),
        side_by_side_rows: render_side_by_side_note_rows(
            request.note_rows,
            request.widths.side_by_side,
            request.note,
        )
        .into_iter()
        .map(RenderRow::note)
        .collect(),
        context: request.context,
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
    fn collapsed_note_rows_truncate_and_expanded_rows_keep_wrapping() {
        let note = Note::new(
            1,
            NoteTarget::File {
                file_path: "test.rs".to_string(),
            },
            "alpha betagammadelta epsilon zeta eta theta iota".to_string(),
        );

        let collapsed = build_note_rows(&note, 14, false);
        let expanded = build_note_rows(&note, 14, true);

        assert_eq!(collapsed.len(), 2);
        assert!(collapsed.last().unwrap().ends_with('…'));
        assert!(expanded.len() > collapsed.len());
    }

    #[test]
    fn agent_threads_show_latest_activity_when_collapsed_and_all_messages_when_expanded() {
        let mut note = Note::new_agent(
            1,
            NoteTarget::File {
                file_path: "test.rs".to_string(),
            },
            crate::note::AgentProvider::Codex,
            "Why is this needed?".to_string(),
            1,
        );
        note.push_agent_message(
            crate::note::AgentProvider::Codex,
            "It protects the shared queue.".to_string(),
        );
        note.push_user_message("Does it cover the worker too?".to_string());
        note.push_agent_message(
            crate::note::AgentProvider::Codex,
            "No, the guard is released first.".to_string(),
        );
        note.agent.as_mut().unwrap().status = AgentStatus::Complete;
        note.agent.as_mut().unwrap().session_id = Some("session-1".to_string());

        let collapsed = build_note_rows(&note, 60, false);
        let expanded = build_note_rows(&note, 60, true);

        assert_eq!(collapsed[0], "You: Why is this needed?");
        assert_eq!(collapsed[1], "Codex: No, the guard is released first.");
        assert_eq!(collapsed[2], "2 more messages");
        assert_eq!(collapsed[3], "Enter to expand · n to reply");
        assert_eq!(expanded.last().unwrap(), "Enter to collapse · n to reply");
        assert_eq!(
            expanded.iter().filter(|row| row.as_str() == "You:").count(),
            2
        );
        assert_eq!(
            expanded
                .iter()
                .filter(|row| row.as_str() == "Codex:")
                .count(),
            2
        );
    }

    #[test]
    fn expanded_agent_reply_keeps_the_end_of_a_wrapped_response() {
        let mut note = Note::new_agent(
            1,
            NoteTarget::File {
                file_path: "test.rs".to_string(),
            },
            crate::note::AgentProvider::Codex,
            "Explain this".to_string(),
            1,
        );
        note.push_agent_message(
            crate::note::AgentProvider::Codex,
            "This response wraps across several lines and still reaches THE_END".to_string(),
        );
        note.agent.as_mut().unwrap().status = AgentStatus::Complete;

        let collapsed = build_note_rows(&note, 24, false);
        let expanded = build_note_rows(&note, 24, true);

        assert!(collapsed.iter().any(|row| row.contains("Enter to expand")));
        assert!(expanded.iter().any(|row| row.contains("THE_END")));
    }

    #[test]
    fn one_sided_agent_reply_wraps_to_its_side_by_side_card_width() {
        let mut note = Note::new_agent(
            1,
            NoteTarget::Line {
                file_path: "test.rs".to_string(),
                old_lineno: None,
                new_lineno: Some(2),
            },
            crate::note::AgentProvider::Codex,
            "What does this do?".to_string(),
            1,
        );
        note.push_agent_message(
            crate::note::AgentProvider::Codex,
            "This response is longer than one side of the diff but must wrap and retain THE_END"
                .to_string(),
        );
        note.agent.as_mut().unwrap().status = AgentStatus::Complete;
        let widths = LayoutWidths {
            inline: 80,
            side_by_side: 80,
        };

        let rows = build_note_rows(&note, note_wrap_width(&note, widths), true);
        let rendered = render_side_by_side_note_rows(&rows, widths.side_by_side, &note);
        let rendered_text = rendered
            .iter()
            .map(plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rows.iter().any(|row| row.contains("THE_END")));
        assert!(rendered_text.contains("THE_END"));
        assert!(rows.len() > 6);
    }

    #[test]
    fn agent_activity_glyph_alternates_between_plus_and_minus() {
        assert_eq!(agent_activity_glyph(std::time::Duration::ZERO), "+");
        assert_eq!(agent_activity_glyph(AGENT_ACTIVITY_INTERVAL), "−");
        assert_eq!(agent_activity_glyph(AGENT_ACTIVITY_INTERVAL * 2), "+");
    }

    #[test]
    fn running_agent_keeps_subtle_actions_in_collapsed_and_expanded_views() {
        let mut note = Note::new_agent(
            1,
            NoteTarget::File {
                file_path: "test.rs".to_string(),
            },
            crate::note::AgentProvider::Claude,
            "Explain this".to_string(),
            1,
        );
        note.agent.as_mut().unwrap().status = AgentStatus::Running {
            started_at: std::time::Instant::now(),
            slow: false,
        };

        let collapsed = build_note_rows(&note, 60, false);
        let expanded = build_note_rows(&note, 60, true);

        assert!(collapsed.iter().any(|row| row.contains("Responding")));
        assert_eq!(collapsed.last().unwrap(), "Enter to expand · c to cancel");
        assert_eq!(expanded.last().unwrap(), "Enter to collapse · c to cancel");
    }

    #[test]
    fn side_by_side_notes_render_on_the_changed_side() {
        let removed_note = Note::new(
            1,
            NoteTarget::Line {
                file_path: "test.rs".to_string(),
                old_lineno: Some(1),
                new_lineno: None,
            },
            "removed".to_string(),
        );
        let added_note = Note::new(
            2,
            NoteTarget::Line {
                file_path: "test.rs".to_string(),
                old_lineno: None,
                new_lineno: Some(2),
            },
            "added".to_string(),
        );

        let removed_line = plain_text(
            &render_side_by_side_note_rows(&["removed".to_string()], 30, &removed_note)[1],
        );
        let added_line =
            plain_text(&render_side_by_side_note_rows(&["added".to_string()], 30, &added_note)[1]);

        assert!(removed_line.starts_with("│ removed"));
        assert!(removed_line.ends_with("               "));
        assert!(added_line.starts_with("              │ "));
        assert!(added_line.contains("│ added"));
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

    fn plain_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
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
