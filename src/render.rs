use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout as FrameLayout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::layout::{Layout as DiffLayout, NodeStatus, RowViewState};
use crate::log;
use crate::note::NoteTarget;
use crate::state::{App, DiffMode, FocusPane, SidebarEntry, SidebarEntryKind};

const OVERSCAN_MULTIPLIER: usize = 2;

pub fn ensure_layout(app: &mut App, area: Rect) {
    let (inline_width, side_width) = render_widths(app, area);
    let viewport_rows = viewport_line_capacity(app, area).max(1);
    let overscan_rows = viewport_rows.saturating_mul(OVERSCAN_MULTIPLIER);
    let needs_rebuild = app.layout.as_ref().is_none_or(|layout| {
        layout.inline_width != inline_width || layout.side_by_side_width != side_width
    });
    let previous_visual_offset = app
        .main_pane
        .cursor_row
        .saturating_sub(app.main_pane.scroll as usize);
    let previous_cursor = app.main_pane.cursor_target.or_else(|| {
        app.layout
            .as_ref()
            .and_then(|layout| layout.row_contexts.get(app.main_pane.cursor_row).copied())
    });

    if needs_rebuild {
        app.layout = Some(DiffLayout::build(
            &app.session,
            &app.notes.items,
            &app.notes.expanded_ids,
            inline_width,
            side_width,
            app.main_pane.selected_file,
            app.main_pane.selected_hunk,
            viewport_rows,
            overscan_rows,
        ));
    } else if let Some(layout) = &mut app.layout {
        let _ = layout.ensure_hunk_window(
            &app.layout_worker,
            &app.session,
            &app.notes.items,
            &app.notes.expanded_ids,
            app.main_pane.selected_file,
            app.main_pane.selected_hunk,
            viewport_rows,
            overscan_rows,
        );
    }

    if let Some(context) = previous_cursor
        && let Some(layout) = &app.layout
    {
        if let Some(index) = row_index_for_context(layout, context) {
            app.main_pane.cursor_row = index;
            app.main_pane.cursor_target = Some(context);
            app.main_pane.scroll = index.saturating_sub(previous_visual_offset) as u16;
        } else if let Some(range) = layout.hunk_ranges.iter().find(|range| {
            range.file_index == app.main_pane.selected_file
                && range.hunk_index == app.main_pane.selected_hunk
        }) {
            app.main_pane.cursor_row = range.start;
            app.main_pane.cursor_target = layout.row_contexts.get(range.start).copied();
            app.main_pane.selection_anchor = None;
            app.main_pane.scroll = range.start.saturating_sub(previous_visual_offset) as u16;
        } else {
            app.main_pane.cursor_target = Some(context);
        }
    }
}

pub fn max_scroll(app: &App, area: Rect) -> u16 {
    let Some(layout) = &app.layout else {
        return 0;
    };

    let visible_lines = viewport_line_capacity(app, area);
    let total_lines = layout.line_count_for_mode(matches!(app.global.mode, DiffMode::SideBySide));
    total_lines.saturating_sub(visible_lines) as u16
}

pub fn max_cursor_row(app: &App) -> usize {
    let Some(layout) = &app.layout else {
        return 0;
    };

    layout
        .line_count_for_mode(matches!(app.global.mode, DiffMode::SideBySide))
        .saturating_sub(1)
}

pub fn reveal_selected_hunk(app: &mut App, area: Rect) {
    if let Some(layout) = &mut app.layout {
        let _ = layout.ensure_selected_hunk_ready_sync(
            &app.session,
            &app.notes.items,
            &app.notes.expanded_ids,
            app.main_pane.selected_file,
            app.main_pane.selected_hunk,
        );
    }

    let Some(layout) = &app.layout else {
        return;
    };

    let Some(range) = layout.hunk_ranges.iter().find(|range| {
        range.file_index == app.main_pane.selected_file
            && range.hunk_index == app.main_pane.selected_hunk
    }) else {
        return;
    };

    app.main_pane.cursor_row = range.start;
    ensure_cursor_visible(app, area);
}

pub fn ensure_cursor_visible(app: &mut App, area: Rect) {
    let visible_lines = viewport_line_capacity(app, area);
    let cursor_row = app.main_pane.cursor_row as u16;

    if cursor_row < app.main_pane.scroll {
        app.main_pane.scroll = cursor_row;
    } else if visible_lines > 0 {
        let bottom = app.main_pane.scroll as usize + visible_lines.saturating_sub(1);
        if app.main_pane.cursor_row > bottom {
            app.main_pane.scroll =
                app.main_pane
                    .cursor_row
                    .saturating_sub(visible_lines.saturating_sub(1)) as u16;
        }
    }

    app.main_pane.scroll = app.main_pane.scroll.min(max_scroll(app, area));
}

pub fn render(frame: &mut Frame<'_>, app: &App) {
    render_body(frame, frame.area(), app);
}

fn render_body(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let chunks = if app.sidebar.open {
        FrameLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(1)])
            .split(area)
    } else {
        FrameLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(0), Constraint::Min(1)])
            .split(area)
    };

    if app.sidebar.open {
        render_sidebar(frame, chunks[0], app);
    }

    render_diff_shell(frame, chunks[1], app);
    render_note_composer(frame, chunks[1], app);
}

fn render_sidebar(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let sections = if app.global.debug_pane_open {
        FrameLayout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(67), Constraint::Percentage(33)])
            .split(area)
    } else {
        FrameLayout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(0)])
            .split(area)
    };

    render_sidebar_files(frame, sections[0], app);
    if app.global.debug_pane_open {
        render_debug_pane(frame, sections[1], app);
    }
}

fn render_sidebar_files(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let rows = app.visible_sidebar_entries();
    let items: Vec<ListItem<'_>> = rows
        .iter()
        .map(|row| {
            ListItem::new(Line::from(Span::styled(
                row.label.clone(),
                sidebar_entry_style(app, row),
            )))
        })
        .collect();

    let mut state = ListState::default().with_selected(Some(app.sidebar.cursor));

    let list = List::new(items)
        .block(pane_block(" Files ", app.global.focus == FocusPane::Files))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol(if app.global.focus == FocusPane::Files {
            ">"
        } else {
            "·"
        });

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_debug_pane(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let lines = debug_lines(app, area.width.saturating_sub(2) as usize);
    let title = if app.global.focus == FocusPane::Files {
        " Debug "
    } else {
        " Debug · D to toggle "
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(pane_block(title, false))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_diff_shell(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.global.mode {
        DiffMode::SideBySide => render_side_by_side(frame, area, app),
        DiffMode::Inline => render_inline(frame, area, app),
    }
}

fn render_side_by_side(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if app.session.files.is_empty() {
        frame.render_widget(
            Paragraph::new("No changes").block(pane_block("", app.global.focus == FocusPane::Main)),
            area,
        );
        return;
    }

    let Some(layout) = &app.layout else {
        return;
    };

    let lines = layout.materialize_rows(
        true,
        &row_view_state(app, app.global.focus == FocusPane::Main),
    );
    frame.render_widget(
        Paragraph::new(lines).block(pane_block("", app.global.focus == FocusPane::Main)),
        area,
    );
}

fn render_inline(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if app.session.files.is_empty() {
        frame.render_widget(
            Paragraph::new("No changes in working tree")
                .block(pane_block("", app.global.focus == FocusPane::Main)),
            area,
        );
        return;
    }

    let Some(layout) = &app.layout else {
        return;
    };

    let lines = layout.materialize_rows(
        false,
        &row_view_state(app, app.global.focus == FocusPane::Main),
    );
    frame.render_widget(
        Paragraph::new(lines).block(pane_block("", app.global.focus == FocusPane::Main)),
        area,
    );
}

fn render_note_composer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(draft) = &app.notes.draft else {
        return;
    };
    let target = app.composer_note_target();

    let inner = inner_pane_area(area);
    if inner.width < 12 || inner.height < 3 {
        return;
    }

    let composer_height = 3;
    let anchor_visible_row = app
        .note_anchor_row()
        .saturating_sub(app.main_pane.scroll as usize) as u16;
    let y = if anchor_visible_row >= composer_height {
        inner.y + anchor_visible_row - composer_height
    } else {
        inner.y + anchor_visible_row.saturating_add(1)
    }
    .min(inner.y + inner.height.saturating_sub(composer_height));

    let width = composer_width(app.global.mode, inner, target.as_ref());
    let x = composer_x(app.global.mode, inner, target.as_ref(), width);
    let area = Rect {
        x,
        y,
        width,
        height: composer_height,
    };

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Line::from(draft.clone())).block(
            Block::default()
                .title(Span::styled(
                    " Note ",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::White)),
        ),
        area,
    );

    let cursor_x = area.x.saturating_add(1).saturating_add(
        draft
            .chars()
            .count()
            .min(area.width.saturating_sub(3) as usize) as u16,
    );
    let cursor_y = area.y.saturating_add(1);
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn composer_width(mode: DiffMode, inner: Rect, target: Option<&NoteTarget>) -> u16 {
    match (mode, composer_side(target)) {
        (DiffMode::SideBySide, ComposerSide::Left | ComposerSide::Right) => {
            let (left_width, right_width) = split_composer_width(inner.width.saturating_sub(3));
            left_width.max(right_width).min(64)
        }
        _ => inner.width.min(64),
    }
}

fn composer_x(mode: DiffMode, inner: Rect, target: Option<&NoteTarget>, width: u16) -> u16 {
    match (mode, composer_side(target)) {
        (DiffMode::SideBySide, ComposerSide::Left) => inner.x,
        (DiffMode::SideBySide, ComposerSide::Right) => {
            let (left_width, _) = split_composer_width(inner.width.saturating_sub(3));
            inner.x + left_width + 3
        }
        _ => inner.x,
    }
    .min(inner.x + inner.width.saturating_sub(width))
}

fn split_composer_width(width: u16) -> (u16, u16) {
    let left = width / 2;
    let right = width.saturating_sub(left);
    (left, right)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ComposerSide {
    Full,
    Left,
    Right,
}

fn composer_side(target: Option<&NoteTarget>) -> ComposerSide {
    match target {
        Some(NoteTarget::Line {
            old_lineno: Some(_),
            new_lineno: None,
            ..
        })
        | Some(NoteTarget::Range {
            start_old_lineno: Some(_),
            start_new_lineno: None,
            ..
        }) => ComposerSide::Left,
        Some(NoteTarget::Line {
            old_lineno: None,
            new_lineno: Some(_),
            ..
        })
        | Some(NoteTarget::Range {
            start_old_lineno: None,
            start_new_lineno: Some(_),
            ..
        }) => ComposerSide::Right,
        _ => ComposerSide::Full,
    }
}

fn render_widths(app: &App, area: Rect) -> (usize, usize) {
    let content = content_area(app, area);
    let inline_width = content.width.saturating_sub(2) as usize;
    let side_by_side_width = content.width.saturating_sub(2) as usize;
    (inline_width, side_by_side_width)
}

fn viewport_line_capacity(app: &App, area: Rect) -> usize {
    content_area(app, area).height.saturating_sub(2) as usize
}

fn content_area(app: &App, area: Rect) -> Rect {
    if app.sidebar.open {
        FrameLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(1)])
            .split(area)[1]
    } else {
        area
    }
}

fn inner_pane_area(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

fn pane_block<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let style = if focused {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    Block::default()
        .title(Span::styled(title, style))
        .borders(Borders::ALL)
        .border_style(style)
}

fn sidebar_entry_style(app: &App, entry: &SidebarEntry) -> Style {
    match entry.kind {
        SidebarEntryKind::Directory { .. } => Style::default().add_modifier(Modifier::BOLD),
        SidebarEntryKind::File { file_index } if file_index == app.main_pane.selected_file => {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        }
        SidebarEntryKind::File { .. } => Style::default(),
    }
}

fn debug_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let events = log::recent_events(32);

    if let Some(rss_mb) = latest_field(&events, "rss_mb") {
        lines.push(Line::from(format!("Memory {}MB", rss_mb)));
    }
    if let Some(layout) = &app.layout {
        let ready_hunks = layout
            .base
            .tree
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .filter(|hunk| hunk.status == NodeStatus::Ready)
            .count();
        let pending_hunks = layout
            .base
            .tree
            .files
            .iter()
            .flat_map(|file| file.hunks.iter())
            .filter(|hunk| hunk.status != NodeStatus::Ready)
            .count();
        lines.push(Line::from(format!("Ready Hunks {}", ready_hunks)));
        lines.push(Line::from(format!("Pending Hunks {}", pending_hunks)));
        lines.push(Line::from(format!("Base Rows {}", layout.base.row_contexts.len())));
    }

    if !lines.is_empty() {
        lines.push(Line::default());
    }

    for name in [
        "diff_load",
        "layout_build_base",
        "layout_refresh_notes",
        "layout_build",
        "first_frame",
    ] {
        if let Some(event) = events.iter().find(|event| event.name == name) {
            lines.push(Line::from(format_debug_event(event, width)));
        }
    }

    if !lines.is_empty() {
        lines.push(Line::default());
    }

    for event in events.iter().take(5) {
        lines.push(Line::from(format_debug_event(&event, width)));
    }

    if lines.is_empty() {
        lines.push(Line::from("No debug events yet"));
    }

    lines
}

fn format_debug_event(event: &log::Event, width: usize) -> String {
    let elapsed_ms = event
        .fields
        .iter()
        .find(|(key, _)| key == "elapsed_ms")
        .map(|(_, value)| value.as_str())
        .unwrap_or("?");

    let mut line = format!("{} {}ms", short_name(&event.name), elapsed_ms);
    for fragment in debug_fragments(event) {
        if line.len() + fragment.len() > width.max(12) {
            break;
        }
        line.push_str(&fragment);
    }
    line
}

fn latest_field<'a>(events: &'a [log::Event], key: &str) -> Option<&'a str> {
    events
        .iter()
        .find_map(|event| {
            event
                .fields
                .iter()
                .find(|(field_key, _)| field_key == key)
                .map(|(_, value)| value.as_str())
        })
}

fn debug_fragments(event: &log::Event) -> Vec<String> {
    if event.name == "layout_expand" {
        let mut fragments = Vec::new();
        for (key, short) in [
            ("build_ms", "b"),
            ("flatten_ms", "f"),
            ("note_ms", "n"),
            ("built_hunks", "bh"),
            ("evicted_hunks", "eh"),
            ("missing_hunks", "mh"),
            ("extra_hunks", "xh"),
        ] {
            if let Some(value) = field_value(event, key) {
                fragments.push(format!(" {short}={value}"));
            }
        }
        return fragments;
    }

    event
        .fields
        .iter()
        .filter(|(key, _)| key != "elapsed_ms")
        .map(|(key, value)| format!(" {key}={value}"))
        .collect()
}

fn field_value<'a>(event: &'a log::Event, key: &str) -> Option<&'a str> {
    event
        .fields
        .iter()
        .find(|(field_key, _)| field_key == key)
        .map(|(_, value)| value.as_str())
}

fn short_name(name: &str) -> &str {
    match name {
        "layout_build_base" => "Base Layout",
        "layout_refresh_notes" => "Note Overlay",
        "layout_build" => "Total Layout",
        "layout_expand" => "Expand",
        "diff_load" => "Diff Load",
        "first_frame" => "First Paint",
        _ => name,
    }
}

fn row_view_state(app: &App, cursor_focused: bool) -> RowViewState {
    RowViewState {
        scroll: app.main_pane.scroll,
        selected_file: app.main_pane.selected_file,
        selected_hunk: app.main_pane.selected_hunk,
        cursor_row: app.main_pane.cursor_row,
        cursor_focused,
        selected_rows: app.selected_row_range(),
    }
}

fn row_index_for_context(layout: &DiffLayout, target: crate::layout::RowContext) -> Option<usize> {
    layout.row_contexts.iter().position(|context| {
        context.file_index == target.file_index
            && context.hunk_index == target.hunk_index
            && context.kind == target.kind
            && context.old_lineno == target.old_lineno
            && context.new_lineno == target.new_lineno
            && context.note_id == target.note_id
    })
}
