use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout as FrameLayout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::layout::{Layout as DiffLayout, RowViewState};
use crate::note::NoteTarget;
use crate::state::{App, DiffMode, FocusPane, SidebarEntry, SidebarEntryKind};

pub fn ensure_layout(app: &mut App, area: Rect) {
    let (inline_width, side_width) = render_widths(app, area);
    let needs_rebuild = app.layout.as_ref().is_none_or(|layout| {
        layout.inline_width != inline_width || layout.side_by_side_width != side_width
    });

    if needs_rebuild {
        app.layout = Some(DiffLayout::build(
            &app.session,
            &app.notes.items,
            &app.notes.expanded_ids,
            inline_width,
            side_width,
        ));
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
