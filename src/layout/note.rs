use std::ops::Range;
use std::time::Duration;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::layout::lines::{combined_side_line, split_side_by_side_width};
use crate::layout::text::{truncate_with_ellipsis, wrap_text};
use crate::note::{
    AGENT_ACTIVITY_INTERVAL, AgentFailureKind, AgentProvider, AgentRunState, AgentThread,
    MessageAuthor, Note, NoteTarget,
};

pub(crate) const NOTE_COMPOSER_HEIGHT: usize = 3;

const CODEX_COLOR: Color = Color::Rgb(180, 120, 255);
const CLAUDE_COLOR: Color = Color::Rgb(255, 159, 67);
const ACTIVITY_PLUS_COLOR: Color = Color::Rgb(80, 200, 120);
const ACTIVITY_MINUS_COLOR: Color = Color::Rgb(255, 95, 95);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiffSide {
    Full,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NoteAuthor {
    User,
    Agent(AgentProvider),
    System,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NoteStatus {
    Waiting,
    Responding { elapsed: Duration, slow: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NoteAction {
    Expand,
    Collapse,
    Reply,
    CancelRun,
    Retry,
    Send,
    CancelComposer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum NoteRow {
    Author(NoteAuthor),
    Summary { author: NoteAuthor, body: String },
    Body(String),
    Status(NoteStatus),
    Metadata(String),
    Hint(Vec<NoteAction>),
    Spacer,
    ComposerSlot,
}

pub(super) struct RenderedNote {
    pub rows: Vec<Line<'static>>,
    pub composer_bounds: Option<Range<usize>>,
}

pub(super) fn build_note_rows(
    note: &Note,
    width: usize,
    expanded: bool,
    composer_active: bool,
) -> Vec<NoteRow> {
    let body_width = width.saturating_sub(4).max(8);
    let Some(thread) = note.agent_thread() else {
        let mut rows = wrap_note_body(note.personal_body().unwrap_or_default(), body_width);
        if rows.is_empty() {
            rows.push(NoteRow::Body(String::new()));
        }
        if !expanded && rows.len() > 2 {
            rows.truncate(2);
            if let Some(NoteRow::Body(last)) = rows.last_mut() {
                *last = truncate_with_ellipsis(last, body_width);
            }
        }
        return rows;
    };

    if expanded {
        build_expanded_agent_rows(thread, body_width, composer_active)
    } else {
        build_collapsed_agent_rows(thread, body_width)
    }
}

fn build_expanded_agent_rows(
    thread: &AgentThread,
    width: usize,
    composer_active: bool,
) -> Vec<NoteRow> {
    let mut rows = Vec::new();
    for message in thread.messages() {
        if !rows.is_empty() {
            rows.push(NoteRow::Spacer);
        }
        let author = match message.author {
            MessageAuthor::User => NoteAuthor::User,
            MessageAuthor::Agent => NoteAuthor::Agent(thread.provider()),
        };
        rows.push(NoteRow::Author(author));
        rows.extend(wrap_note_body(&message.body, width));
    }

    match thread.state() {
        AgentRunState::Queued { .. } => {
            rows.push(NoteRow::Spacer);
            rows.push(NoteRow::Status(NoteStatus::Waiting));
        }
        AgentRunState::Running {
            started_at, slow, ..
        } => {
            rows.push(NoteRow::Spacer);
            rows.push(NoteRow::Status(NoteStatus::Responding {
                elapsed: started_at.elapsed(),
                slow: *slow,
            }));
        }
        AgentRunState::Failed { failure, .. } => {
            rows.push(NoteRow::Spacer);
            rows.push(NoteRow::Author(NoteAuthor::System));
            rows.extend(wrap_note_body(&failure.message, width));
        }
        AgentRunState::Cancelled { .. } => {
            rows.push(NoteRow::Spacer);
            rows.push(NoteRow::Author(NoteAuthor::System));
            rows.push(NoteRow::Body(
                "The agent request was cancelled.".to_string(),
            ));
        }
        AgentRunState::Ready { .. } => {}
    }

    rows.push(NoteRow::Hint(note_actions(
        thread.state(),
        true,
        composer_active,
    )));
    if composer_active {
        rows.push(NoteRow::ComposerSlot);
    }
    rows
}

fn build_collapsed_agent_rows(thread: &AgentThread, _width: usize) -> Vec<NoteRow> {
    let mut rows = Vec::new();
    let messages = thread.messages();
    let show_current_attempt = match thread.state() {
        AgentRunState::Ready { .. } => false,
        AgentRunState::Queued { .. }
        | AgentRunState::Running { .. }
        | AgentRunState::Failed { .. }
        | AgentRunState::Cancelled { .. } => true,
    };

    if show_current_attempt {
        if let Some(message) = messages
            .iter()
            .rev()
            .find(|message| message.author == MessageAuthor::User)
        {
            rows.push(NoteRow::Summary {
                author: NoteAuthor::User,
                body: message.body.clone(),
            });
        }
        rows.push(status_row(thread.state()));
        if messages.len() > 1 {
            rows.push(NoteRow::Metadata(format!(
                "{} earlier messages",
                messages.len() - 1
            )));
        }
        rows.push(NoteRow::Hint(note_actions(thread.state(), false, false)));
        return rows;
    }

    if let Some(message) = messages
        .iter()
        .find(|message| message.author == MessageAuthor::User)
    {
        rows.push(NoteRow::Summary {
            author: NoteAuthor::User,
            body: message.body.clone(),
        });
    }

    if let Some(message) = messages
        .iter()
        .rev()
        .find(|message| message.author == MessageAuthor::Agent)
    {
        rows.push(NoteRow::Summary {
            author: NoteAuthor::Agent(thread.provider()),
            body: message.body.clone(),
        });
    } else {
        rows.push(status_row(thread.state()));
    }

    if messages.len() > 2 {
        rows.push(NoteRow::Metadata(format!(
            "{} more messages",
            messages.len() - 2
        )));
    }
    rows.push(NoteRow::Hint(note_actions(thread.state(), false, false)));
    rows
}

fn status_row(state: &AgentRunState) -> NoteRow {
    match state {
        AgentRunState::Queued { .. } => NoteRow::Status(NoteStatus::Waiting),
        AgentRunState::Running {
            started_at, slow, ..
        } => NoteRow::Status(NoteStatus::Responding {
            elapsed: started_at.elapsed(),
            slow: *slow,
        }),
        AgentRunState::Ready { .. } => NoteRow::Metadata("Answered".to_string()),
        AgentRunState::Failed { failure, .. } if failure.retryable => {
            NoteRow::Metadata("Agent request failed · r to retry".to_string())
        }
        AgentRunState::Failed { .. } => NoteRow::Metadata("Agent request failed".to_string()),
        AgentRunState::Cancelled { .. } => {
            NoteRow::Metadata("Agent request cancelled · r to retry".to_string())
        }
    }
}

fn note_actions(state: &AgentRunState, expanded: bool, composer_active: bool) -> Vec<NoteAction> {
    if composer_active {
        return vec![NoteAction::Send, NoteAction::CancelComposer];
    }

    let mut actions = vec![if expanded {
        NoteAction::Collapse
    } else {
        NoteAction::Expand
    }];
    if state.is_active() {
        actions.push(NoteAction::CancelRun);
    } else if state.can_reply() {
        actions.push(NoteAction::Reply);
    } else if state.can_retry() {
        actions.push(NoteAction::Retry);
    }
    actions
}

fn wrap_note_body(body: &str, width: usize) -> Vec<NoteRow> {
    let mut rows = Vec::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            rows.push(NoteRow::Body(String::new()));
            continue;
        }
        rows.extend(wrap_text(line, width).into_iter().map(NoteRow::Body));
    }
    rows
}

pub(super) fn render_note_rows(note: &Note, rows: &[NoteRow], width: usize) -> RenderedNote {
    let inner_width = width.saturating_sub(2).max(4);
    let border_style = Style::default().fg(Color::DarkGray);
    let mut rendered = Vec::with_capacity(rows.len() + 2);

    let title = note.agent_thread().map(|thread| {
        format!(
            "{} · {}",
            thread.provider().mention(),
            agent_state_label(thread.state())
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

    let mut composer_bounds = None;
    for row in rows {
        if matches!(row, NoteRow::ComposerSlot) {
            let start = rendered.len();
            rendered.extend(
                (0..NOTE_COMPOSER_HEIGHT).map(|_| bordered_line(vec![], inner_width, border_style)),
            );
            composer_bounds = Some(start..rendered.len());
            continue;
        }
        rendered.push(render_note_row(row, inner_width, border_style));
    }

    rendered.push(Line::from(vec![
        Span::styled("└".to_string(), border_style),
        Span::styled("─".repeat(inner_width), border_style),
        Span::styled("┘".to_string(), border_style),
    ]));

    RenderedNote {
        rows: rendered,
        composer_bounds,
    }
}

pub(super) fn render_side_by_side_note_rows(
    note: &Note,
    rows: &[NoteRow],
    width: usize,
) -> RenderedNote {
    let side = note_side(&note.target);
    if side == DiffSide::Full {
        return render_note_rows(note, rows, width);
    }

    let (left_width, right_width) = split_side_by_side_width(width);
    let note_width = width_for_side(side, width);
    let rendered = render_note_rows(note, rows, note_width);
    let divider_style = Style::default().fg(Color::DarkGray);
    let rows = rendered
        .rows
        .into_iter()
        .map(|note_row| match side {
            DiffSide::Left => combined_side_line(note_row, blank_side_line(right_width)),
            DiffSide::Right => {
                let mut spans = blank_side_line(left_width).spans;
                spans.push(Span::styled(" │ ".to_string(), divider_style));
                spans.extend(note_row.spans);
                Line::from(spans)
            }
            DiffSide::Full => unreachable!(),
        })
        .collect();
    RenderedNote {
        rows,
        composer_bounds: rendered.composer_bounds,
    }
}

fn render_note_row(row: &NoteRow, width: usize, border_style: Style) -> Line<'static> {
    let base_style = Style::default().fg(Color::White);
    let (text, spans) = match row {
        NoteRow::Author(author) => {
            let label = author_label(*author);
            let text = format!(" {label}");
            let spans = vec![Span::styled(
                text.clone(),
                author_style(*author, base_style),
            )];
            (text, spans)
        }
        NoteRow::Summary { author, body } => {
            let label = author_label(*author);
            let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
            let text = truncate_with_ellipsis(&format!(" {label} {body}"), width);
            let prefix_len = (label.chars().count() + 2).min(text.len());
            let mut spans = vec![Span::styled(
                text[..prefix_len].to_string(),
                author_style(*author, base_style),
            )];
            spans.extend(styled_mentions(&text[prefix_len..], base_style));
            (text, spans)
        }
        NoteRow::Body(body) => {
            let text = format!(" {body}");
            let spans = styled_mentions(&text, base_style);
            (text, spans)
        }
        NoteRow::Status(status) => {
            let (glyph, label) = match status {
                NoteStatus::Waiting => (None, "Waiting to start…".to_string()),
                NoteStatus::Responding {
                    elapsed,
                    slow: true,
                } => (
                    Some(activity_glyph(*elapsed)),
                    format!("Still responding · {}m", elapsed.as_secs().div_ceil(60)),
                ),
                NoteStatus::Responding { elapsed, .. } => {
                    (Some(activity_glyph(*elapsed)), "Responding…".to_string())
                }
            };
            let text = glyph.map_or_else(
                || format!(" {label}"),
                |(glyph, _)| format!(" {glyph} {label}"),
            );
            let subtle = base_style.fg(Color::DarkGray);
            let spans = if let Some((glyph, color)) = glyph {
                vec![
                    Span::styled(" ".to_string(), subtle),
                    Span::styled(
                        glyph.to_string(),
                        base_style.fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" {label}"), subtle),
                ]
            } else {
                vec![Span::styled(text.clone(), subtle)]
            };
            (text, spans)
        }
        NoteRow::Metadata(metadata) => {
            let text = format!(" {metadata}");
            let spans = vec![Span::styled(text.clone(), base_style.fg(Color::DarkGray))];
            (text, spans)
        }
        NoteRow::Hint(actions) => {
            let label = actions
                .iter()
                .map(|action| action_label(*action))
                .collect::<Vec<_>>()
                .join(" · ");
            let text = format!(" {label}");
            let spans = vec![Span::styled(text.clone(), base_style.fg(Color::DarkGray))];
            (text, spans)
        }
        NoteRow::Spacer => (String::new(), Vec::new()),
        NoteRow::ComposerSlot => unreachable!(),
    };

    bordered_line(
        pad_spans(spans, text.chars().count(), width),
        width,
        border_style,
    )
}

fn bordered_line(spans: Vec<Span<'static>>, width: usize, border_style: Style) -> Line<'static> {
    let mut line = vec![Span::styled("│".to_string(), border_style)];
    line.extend(spans);
    if line_width(&line).saturating_sub(1) < width {
        line.push(Span::raw(" ".repeat(
            width.saturating_sub(line_width(&line).saturating_sub(1)),
        )));
    }
    line.push(Span::styled("│".to_string(), border_style));
    Line::from(line)
}

fn pad_spans(mut spans: Vec<Span<'static>>, text_width: usize, width: usize) -> Vec<Span<'static>> {
    if text_width > width {
        return truncate_spans(spans, width);
    }
    spans.push(Span::raw(" ".repeat(width - text_width)));
    spans
}

fn truncate_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }

    let retained = width.saturating_sub(1);
    let fallback_style = spans.first().map_or_else(Style::default, |span| span.style);
    let mut remaining = retained;
    let mut truncated = Vec::new();
    let mut ellipsis_style = fallback_style;
    for span in spans {
        if remaining == 0 {
            ellipsis_style = span.style;
            break;
        }
        let content = span.content.chars().take(remaining).collect::<String>();
        let count = content.chars().count();
        if count > 0 {
            ellipsis_style = span.style;
            truncated.push(Span::styled(content, span.style));
            remaining -= count;
        }
    }
    truncated.push(Span::styled("…".to_string(), ellipsis_style));
    truncated
}

fn line_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|span| span.content.chars().count()).sum()
}

fn author_label(author: NoteAuthor) -> String {
    match author {
        NoteAuthor::User => "You:".to_string(),
        NoteAuthor::Agent(provider) => format!("{}:", provider.label()),
        NoteAuthor::System => "Enza:".to_string(),
    }
}

fn author_style(author: NoteAuthor, base: Style) -> Style {
    match author {
        NoteAuthor::Agent(provider) => base.fg(provider_color(provider)),
        NoteAuthor::User | NoteAuthor::System => base,
    }
    .add_modifier(Modifier::BOLD)
}

fn action_label(action: NoteAction) -> &'static str {
    match action {
        NoteAction::Expand => "Enter to expand",
        NoteAction::Collapse => "Enter to collapse",
        NoteAction::Reply => "n to reply",
        NoteAction::CancelRun => "c to cancel",
        NoteAction::Retry => "r to retry",
        NoteAction::Send => "Enter to send",
        NoteAction::CancelComposer => "Esc to cancel",
    }
}

fn agent_state_label(state: &AgentRunState) -> &'static str {
    match state {
        AgentRunState::Queued { .. } => "queued",
        AgentRunState::Running { slow: false, .. } => "responding",
        AgentRunState::Running { slow: true, .. } => "still responding",
        AgentRunState::Ready { .. } => "answered",
        AgentRunState::Failed { failure, .. } if failure.kind == AgentFailureKind::Timeout => {
            "timed out"
        }
        AgentRunState::Failed { .. } => "failed",
        AgentRunState::Cancelled { .. } => "cancelled",
    }
}

pub(crate) fn note_side(target: &NoteTarget) -> DiffSide {
    match target {
        NoteTarget::Line {
            old_lineno: Some(_),
            new_lineno: None,
            ..
        }
        | NoteTarget::Range {
            start_old_lineno: Some(_),
            start_new_lineno: None,
            ..
        } => DiffSide::Left,
        NoteTarget::Line {
            old_lineno: None,
            new_lineno: Some(_),
            ..
        }
        | NoteTarget::Range {
            start_old_lineno: None,
            start_new_lineno: Some(_),
            ..
        } => DiffSide::Right,
        _ => DiffSide::Full,
    }
}

pub(crate) fn width_for_side(side: DiffSide, total_width: usize) -> usize {
    let (left_width, right_width) = split_side_by_side_width(total_width);
    match side {
        DiffSide::Full => total_width,
        DiffSide::Left => left_width,
        DiffSide::Right => right_width,
    }
}

pub(super) fn note_wrap_width(note: &Note, inline_width: usize, side_width: usize) -> usize {
    inline_width.min(width_for_side(note_side(&note.target), side_width))
}

fn blank_side_line(width: usize) -> Line<'static> {
    Line::from(Span::raw(" ".repeat(width)))
}

fn activity_glyph(elapsed: Duration) -> (&'static str, Color) {
    let phase = elapsed.as_millis() / AGENT_ACTIVITY_INTERVAL.as_millis();
    if phase.is_multiple_of(2) {
        ("+", ACTIVITY_PLUS_COLOR)
    } else {
        ("−", ACTIVITY_MINUS_COLOR)
    }
}

pub(crate) fn styled_mentions(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let lowercase = text.to_ascii_lowercase();
    let providers = [AgentProvider::Codex, AgentProvider::Claude];
    let mut spans = Vec::new();
    let mut emitted_through = 0;
    let mut search_from = 0;

    while search_from < text.len() {
        let next = providers
            .iter()
            .filter_map(|provider| {
                lowercase[search_from..]
                    .find(provider.mention())
                    .map(|offset| (search_from + offset, *provider))
            })
            .min_by_key(|(index, _)| *index);
        let Some((start, provider)) = next else {
            break;
        };
        let end = start + provider.mention().len();
        if !has_mention_boundaries(text, start, end) {
            search_from = start + 1;
            continue;
        }
        if emitted_through < start {
            spans.push(Span::styled(
                text[emitted_through..start].to_string(),
                base_style,
            ));
        }
        spans.push(Span::styled(
            text[start..end].to_string(),
            base_style.fg(provider_color(provider)),
        ));
        emitted_through = end;
        search_from = end;
    }
    if emitted_through < text.len() {
        spans.push(Span::styled(
            text[emitted_through..].to_string(),
            base_style,
        ));
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base_style));
    }
    spans
}

fn provider_color(provider: AgentProvider) -> Color {
    match provider {
        AgentProvider::Codex => CODEX_COLOR,
        AgentProvider::Claude => CLAUDE_COLOR,
    }
}

fn has_mention_boundaries(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    before.is_none_or(is_mention_boundary) && after.is_none_or(is_mention_boundary)
}

fn is_mention_boundary(character: char) -> bool {
    !character.is_alphanumeric() && character != '_' && character != '@'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::AgentFailure;

    fn file_target() -> NoteTarget {
        NoteTarget::File {
            file_path: "test.rs".to_string(),
        }
    }

    fn ready_agent_note(provider: AgentProvider, response: &str) -> Note {
        let mut note = Note::new_agent(
            1,
            file_target(),
            provider,
            "Why is this needed?".to_string(),
            1,
        );
        let thread = note.agent_thread_mut().unwrap();
        assert!(thread.mark_running(1, std::time::Instant::now()));
        assert!(thread.complete(1, "session-1".to_string(), response.to_string()));
        note
    }

    #[test]
    fn known_mentions_receive_provider_colors() {
        let spans = styled_mentions("Ask @CODEX, then @claude.", Style::default());

        assert_eq!(
            spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "Ask @CODEX, then @claude."
        );
        assert_eq!(
            spans
                .iter()
                .find(|span| span.content == "@CODEX")
                .unwrap()
                .style
                .fg,
            Some(CODEX_COLOR)
        );
        assert_eq!(
            spans
                .iter()
                .find(|span| span.content == "@claude")
                .unwrap()
                .style
                .fg,
            Some(CLAUDE_COLOR)
        );
    }

    #[test]
    fn partial_and_escaped_mentions_are_not_colored() {
        let spans = styled_mentions("@codexchange @@claude", Style::default());

        assert!(spans.iter().all(|span| span.style.fg.is_none()));
    }

    #[test]
    fn activity_glyph_alternates_between_plus_and_minus() {
        assert_eq!(activity_glyph(Duration::ZERO).0, "+");
        assert_eq!(activity_glyph(AGENT_ACTIVITY_INTERVAL).0, "−");
        assert_eq!(activity_glyph(AGENT_ACTIVITY_INTERVAL * 2).0, "+");
    }

    #[test]
    fn personal_notes_truncate_only_when_collapsed() {
        let note = Note::new(
            1,
            file_target(),
            "alpha betagammadelta epsilon zeta eta theta iota".to_string(),
        );

        let collapsed = build_note_rows(&note, 14, false, false);
        let expanded = build_note_rows(&note, 14, true, false);

        assert_eq!(collapsed.len(), 2);
        assert!(matches!(collapsed.last(), Some(NoteRow::Body(body)) if body.ends_with('…')));
        assert!(expanded.len() > collapsed.len());
    }

    #[test]
    fn expanded_agent_responses_wrap_without_losing_the_end() {
        let note = ready_agent_note(
            AgentProvider::Codex,
            "This response wraps across several lines and still reaches THE_END",
        );

        let rows = build_note_rows(&note, 24, true, false);
        let rendered = render_note_rows(&note, &rows, 24);
        let text = rendered
            .rows
            .iter()
            .map(plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("THE_END"));
        assert!(rendered.rows.len() > 6);
    }

    #[test]
    fn collapsed_failed_and_cancelled_follow_ups_show_the_latest_user_message() {
        let mut failed = ready_agent_note(AgentProvider::Codex, "The first answer.");
        let failed_thread = failed.agent_thread_mut().unwrap();
        assert!(failed_thread.queue_reply(2, "The failed follow-up".to_string()));
        assert!(failed_thread.fail(
            2,
            AgentFailure::new(AgentFailureKind::ProcessExit, "The follow-up failed.", true,),
        ));

        let mut cancelled = ready_agent_note(AgentProvider::Claude, "The first answer.");
        let cancelled_thread = cancelled.agent_thread_mut().unwrap();
        assert!(cancelled_thread.queue_reply(2, "The cancelled follow-up".to_string()));
        assert!(cancelled_thread.cancel(2));

        for (note, expected_message, expected_status) in [
            (failed, "The failed follow-up", "Agent request failed"),
            (
                cancelled,
                "The cancelled follow-up",
                "Agent request cancelled",
            ),
        ] {
            let rows = build_note_rows(&note, 60, false, false);
            assert!(matches!(
                &rows[0],
                NoteRow::Summary {
                    author: NoteAuthor::User,
                    body,
                } if body == expected_message
            ));
            assert!(matches!(
                &rows[1],
                NoteRow::Metadata(status) if status.starts_with(expected_status)
            ));
            assert!(!rows.iter().any(|row| matches!(
                row,
                NoteRow::Summary {
                    author: NoteAuthor::Agent(_),
                    ..
                }
            )));
        }
    }

    #[test]
    fn typed_body_rows_do_not_infer_author_styling_from_response_text() {
        let note = ready_agent_note(AgentProvider::Codex, "Codex: this is response text");
        let rows = build_note_rows(&note, 60, true, false);
        let rendered = render_note_rows(&note, &rows, 60);
        let body_line = rendered
            .rows
            .iter()
            .find(|line| plain_text(line).contains("Codex: this is response text"))
            .unwrap();

        assert!(body_line.spans.iter().all(|span| {
            !span.content.contains("Codex:") || !span.style.add_modifier.contains(Modifier::BOLD)
        }));
        let author = rendered
            .rows
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| {
                span.content.contains("Codex:") && span.style.add_modifier.contains(Modifier::BOLD)
            })
            .unwrap();
        assert_eq!(author.style.fg, Some(CODEX_COLOR));
        assert!(author.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn one_sided_notes_use_the_shared_diff_side_geometry() {
        let note = Note::new(
            1,
            NoteTarget::Line {
                file_path: "test.rs".to_string(),
                old_lineno: None,
                new_lineno: Some(2),
            },
            "added".to_string(),
        );
        let rows = build_note_rows(&note, note_wrap_width(&note, 80, 80), true, false);
        let rendered = render_side_by_side_note_rows(&note, &rows, 80);

        assert_eq!(note_side(&note.target), DiffSide::Right);
        assert_eq!(width_for_side(DiffSide::Right, 80), 39);
        assert!(plain_text(&rendered.rows[1]).contains("│ added"));
    }

    fn plain_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }
}
