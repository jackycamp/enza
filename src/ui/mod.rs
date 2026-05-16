use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::app::{App, DiffMode, FocusPane, SidebarEntry, SidebarEntryKind};
use crate::render_cache::{RenderSession, materialize_rows};

pub fn ensure_render_session(app: &mut App, area: Rect) {
    let (inline_width, side_width) = render_widths(app, area);
    let needs_rebuild = app.render_session.as_ref().is_none_or(|cache| {
        cache.inline_width != inline_width || cache.side_by_side_width != side_width
    });

    if needs_rebuild {
        app.render_session = Some(RenderSession::build(&app.session, inline_width, side_width));
    }
}

pub fn max_scroll(app: &App, area: Rect) -> u16 {
    let Some(cache) = &app.render_session else {
        return 0;
    };

    let visible_lines = viewport_line_capacity(app, area);
    let total_lines = cache.line_count_for_mode(matches!(app.mode, DiffMode::SideBySide));
    total_lines.saturating_sub(visible_lines) as u16
}

pub fn sync_selection_to_scroll(app: &mut App) {
    let Some(cache) = &app.render_session else {
        return;
    };

    let top_line = app.scroll as usize;
    if let Some(range) = cache.hunk_ranges.iter().find(|range| top_line < range.end) {
        app.selected_file = range.file_index;
        app.selected_hunk = range.hunk_index;
    }
}

pub fn reveal_selected_hunk(app: &mut App, area: Rect) {
    let Some(cache) = &app.render_session else {
        return;
    };

    let Some(range) = cache.hunk_ranges.iter().find(|range| {
        range.file_index == app.selected_file && range.hunk_index == app.selected_hunk
    }) else {
        return;
    };

    app.scroll = range.start as u16;
    app.scroll = app.scroll.min(max_scroll(app, area));
}

pub fn render(frame: &mut Frame<'_>, app: &App) {
    render_body(frame, frame.area(), app);
}

fn render_body(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let chunks = if app.sidebar_open {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(1)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(0), Constraint::Min(1)])
            .split(area)
    };

    if app.sidebar_open {
        render_sidebar(frame, chunks[0], app);
    }

    render_diff_shell(frame, chunks[1], app);
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

    let mut state = ListState::default().with_selected(Some(app.sidebar_cursor));

    let list = List::new(items)
        .block(pane_block(" Files ", app.focus == FocusPane::Files))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol(if app.focus == FocusPane::Files {
            ">"
        } else {
            "·"
        });

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_diff_shell(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.mode {
        DiffMode::SideBySide => render_side_by_side(frame, area, app),
        DiffMode::Inline => render_inline(frame, area, app),
    }
}

fn render_side_by_side(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if app.session.files.is_empty() {
        frame.render_widget(
            Paragraph::new("No changes").block(pane_block("", app.focus == FocusPane::Main)),
            area,
        );
        return;
    }

    let Some(cache) = &app.render_session else {
        return;
    };

    let lines = materialize_rows(
        &cache.side_by_side_rows,
        app.scroll,
        app.selected_file,
        app.selected_hunk,
    );
    frame.render_widget(
        Paragraph::new(lines).block(pane_block("", app.focus == FocusPane::Main)),
        area,
    );
}

fn render_inline(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if app.session.files.is_empty() {
        frame.render_widget(
            Paragraph::new("No changes in working tree")
                .block(pane_block("", app.focus == FocusPane::Main)),
            area,
        );
        return;
    }

    let Some(cache) = &app.render_session else {
        return;
    };

    let lines = materialize_rows(
        &cache.inline_rows,
        app.scroll,
        app.selected_file,
        app.selected_hunk,
    );
    frame.render_widget(
        Paragraph::new(lines).block(pane_block("", app.focus == FocusPane::Main)),
        area,
    );
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
    if app.sidebar_open {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(1)])
            .split(area)[1]
    } else {
        area
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
        SidebarEntryKind::File { file_index } if file_index == app.selected_file => {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        }
        SidebarEntryKind::File { .. } => Style::default(),
    }
}
