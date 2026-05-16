use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::app::{App, DiffMode, FocusPane, SidebarEntry, SidebarEntryKind};
use crate::render_cache::{RenderSession, materialize_rows};

pub fn ensure_render_session(app: &mut App, area: Rect) {
    let (inline_width, side_width) = render_widths(app, area);
    let needs_rebuild = app
        .render_session
        .as_ref()
        .is_none_or(|cache| cache.inline_width != inline_width || cache.side_width != side_width);

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
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(frame.area());

    render_header(frame, root[0], app);
    render_body(frame, root[1], app);
    render_footer(frame, root[2], app);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let line = Line::from(vec![
        Span::styled("enza", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::raw(format!("mode: {}", app.mode.label())),
        Span::raw("  "),
        Span::raw(format!(
            "sidebar: {}",
            if app.sidebar_open { "open" } else { "closed" }
        )),
        Span::raw("  "),
        Span::raw(format!("focus: {}", app.focus.label())),
        Span::raw("  "),
        Span::raw(format!("file: {}", app.selected_file_name())),
        Span::raw("  "),
        Span::raw(format!(
            "hunk: {}/{}",
            app.selected_hunk_global_index(),
            app.total_hunks()
        )),
    ]);

    frame.render_widget(Paragraph::new(line), area);
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
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    if app.session.files.is_empty() {
        frame.render_widget(
            Paragraph::new("No changes").block(pane_block(" Old ", app.focus == FocusPane::Main)),
            panes[0],
        );
        frame.render_widget(
            Paragraph::new("No changes").block(pane_block(" New ", app.focus == FocusPane::Main)),
            panes[1],
        );
        return;
    }

    let Some(cache) = &app.render_session else {
        return;
    };

    let old_lines = materialize_rows(
        &cache.old_rows,
        app.scroll,
        app.selected_file,
        app.selected_hunk,
    );
    let new_lines = materialize_rows(
        &cache.new_rows,
        app.scroll,
        app.selected_file,
        app.selected_hunk,
    );

    frame.render_widget(
        Paragraph::new(old_lines).block(pane_block(" Old ", app.focus == FocusPane::Main)),
        panes[0],
    );
    frame.render_widget(
        Paragraph::new(new_lines).block(pane_block(" New ", app.focus == FocusPane::Main)),
        panes[1],
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

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let text = vec![
        Line::from(vec![
            "q".bold(),
            " quit  ".into(),
            "j/k".bold(),
            " scroll  ".into(),
            "]/[".bold(),
            " next/prev hunk  ".into(),
            "ctrl-d/u".bold(),
            " page scroll  ".into(),
            "tab".bold(),
            " next focus  ".into(),
            "shift-tab".bold(),
            " prev focus  ".into(),
            "enter".bold(),
            " toggle/jump  ".into(),
            "left/right".bold(),
            " collapse/expand  ".into(),
            "m".bold(),
            " toggle mode  ".into(),
            "b".bold(),
            " toggle sidebar".into(),
        ]),
        Line::from(format!(
            "current: mode={} focus={} sidebar={} file={} hunk={} ({}) scroll={}",
            app.mode.label(),
            app.focus.label(),
            if app.sidebar_open { "open" } else { "closed" },
            app.selected_file_name(),
            app.selected_hunk_number(),
            app.selected_hunk_header(),
            app.scroll
        )),
    ];

    frame.render_widget(Paragraph::new(text), area);
}

fn render_widths(app: &App, area: Rect) -> (usize, usize) {
    let content = content_area(app, area);
    let inline_width = content.width.saturating_sub(2) as usize;
    let side_panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(content);
    let side_width = side_panes[0].width.saturating_sub(2) as usize;
    (inline_width, side_width)
}

fn viewport_line_capacity(app: &App, area: Rect) -> usize {
    content_area(app, area).height.saturating_sub(2) as usize
}

fn content_area(app: &App, area: Rect) -> Rect {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);
    if app.sidebar_open {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(1)])
            .split(root[1])[1]
    } else {
        root[1]
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
