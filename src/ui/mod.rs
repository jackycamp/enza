use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::app::{App, DiffMode, FocusPane, SidebarEntry, SidebarEntryKind};
use crate::diff::{DiffFile, DiffLine};
use crate::highlight::{DiffKind, FileHighlighter};

#[derive(Clone, Copy)]
struct HunkRange {
    file_index: usize,
    hunk_index: usize,
    start: usize,
    end: usize,
}

pub fn max_scroll(app: &App, area: Rect) -> u16 {
    let visible_lines = viewport_line_capacity(app, area);
    let total_lines = document_line_count(app);
    total_lines.saturating_sub(visible_lines) as u16
}

pub fn sync_selection_to_scroll(app: &mut App) {
    let top_line = app.scroll as usize;
    if let Some(range) = document_hunk_ranges(app)
        .into_iter()
        .find(|range| top_line < range.end)
    {
        app.selected_file = range.file_index;
        app.selected_hunk = range.hunk_index;
    }
}

pub fn reveal_selected_hunk(app: &mut App, area: Rect) {
    let Some(range) = document_hunk_ranges(app).into_iter().find(|range| {
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

    let old_lines = side_session_lines(
        app,
        app.scroll,
        panes[0].width.saturating_sub(2) as usize,
        DiffSide::Old,
    );
    let new_lines = side_session_lines(
        app,
        app.scroll,
        panes[1].width.saturating_sub(2) as usize,
        DiffSide::New,
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
                .block(pane_block(" Inline Session ", app.focus == FocusPane::Main)),
            area,
        );
        return;
    }

    let lines = inline_session_lines(app, app.scroll, area.width.saturating_sub(2) as usize);
    frame.render_widget(
        Paragraph::new(lines).block(pane_block(" Inline Session ", app.focus == FocusPane::Main)),
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

fn document_line_count(app: &App) -> usize {
    app.session
        .files
        .iter()
        .map(|file| {
            let per_hunk = file
                .hunks
                .iter()
                .map(|hunk| {
                    let content_lines = hunk.lines.len();
                    let header_lines = match app.mode {
                        DiffMode::Inline | DiffMode::SideBySide => 1,
                    };
                    header_lines + content_lines + 1
                })
                .sum::<usize>();
            2 + per_hunk + 1
        })
        .sum()
}

fn viewport_line_capacity(app: &App, area: Rect) -> usize {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);
    let body = if app.sidebar_open {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(1)])
            .split(root[1])[1]
    } else {
        root[1]
    };

    body.height.saturating_sub(2) as usize
}

fn document_hunk_ranges(app: &App) -> Vec<HunkRange> {
    let mut ranges = Vec::new();
    let mut cursor = 0usize;

    for (file_index, file) in app.session.files.iter().enumerate() {
        cursor += 2;

        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let start = cursor;
            let end = start + 1 + hunk.lines.len() + 1;
            ranges.push(HunkRange {
                file_index,
                hunk_index,
                start,
                end,
            });
            cursor = end;
        }

        cursor += 1;
    }

    ranges
}

fn inline_session_lines<'a>(app: &'a App, scroll: u16, width: usize) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    for (file_index, file) in app.session.files.iter().enumerate() {
        let mut highlighter = FileHighlighter::new(&file.path);
        lines.push(file_separator_line(width));
        lines.push(file_header_line(file, file_index == app.selected_file));

        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let selected = file_index == app.selected_file && hunk_index == app.selected_hunk;
            lines.push(hunk_header_line(&hunk.header, selected));

            for diff_line in &hunk.lines {
                lines.push(match diff_line {
                    DiffLine::Context {
                        old_lineno,
                        new_lineno,
                        text,
                    } => highlighted_prefixed_line(
                        " ",
                        Some(*old_lineno),
                        Some(*new_lineno),
                        text,
                        None,
                        width,
                        &mut highlighter,
                        DiffKind::Context,
                    ),
                    DiffLine::Added { new_lineno, text } => highlighted_prefixed_line(
                        "+",
                        None,
                        Some(*new_lineno),
                        text,
                        Some(Color::Green),
                        width,
                        &mut highlighter,
                        DiffKind::Added,
                    ),
                    DiffLine::Removed { old_lineno, text } => highlighted_prefixed_line(
                        "-",
                        Some(*old_lineno),
                        None,
                        text,
                        Some(Color::Red),
                        width,
                        &mut highlighter,
                        DiffKind::Removed,
                    ),
                });
            }

            lines.push(Line::default());
        }
        lines.push(Line::default());
    }

    lines.into_iter().skip(scroll as usize).collect()
}

#[derive(Clone, Copy)]
enum DiffSide {
    Old,
    New,
}

fn side_session_lines<'a>(
    app: &'a App,
    scroll: u16,
    width: usize,
    side: DiffSide,
) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    for (file_index, file) in app.session.files.iter().enumerate() {
        let mut highlighter = FileHighlighter::new(&file.path);
        lines.push(file_separator_line(width));
        lines.push(file_side_header_line(
            file,
            file_index == app.selected_file,
            side,
        ));

        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let selected = file_index == app.selected_file && hunk_index == app.selected_hunk;
            lines.push(hunk_header_line(&hunk.header, selected));

            for diff_line in &hunk.lines {
                lines.push(match diff_line {
                    DiffLine::Context {
                        old_lineno,
                        new_lineno,
                        text,
                    } => match side {
                        DiffSide::Old => highlighted_side_line(
                            " ",
                            Some(*old_lineno),
                            text,
                            width,
                            None,
                            &mut highlighter,
                            DiffKind::Context,
                        ),
                        DiffSide::New => highlighted_side_line(
                            " ",
                            Some(*new_lineno),
                            text,
                            width,
                            None,
                            &mut highlighter,
                            DiffKind::Context,
                        ),
                    },
                    DiffLine::Added { new_lineno, text } => match side {
                        DiffSide::Old => side_line(" ", None, "", width, Some(Color::DarkGray)),
                        DiffSide::New => highlighted_side_line(
                            "+",
                            Some(*new_lineno),
                            text,
                            width,
                            Some(Color::Green),
                            &mut highlighter,
                            DiffKind::Added,
                        ),
                    },
                    DiffLine::Removed { old_lineno, text } => match side {
                        DiffSide::Old => highlighted_side_line(
                            "-",
                            Some(*old_lineno),
                            text,
                            width,
                            Some(Color::Red),
                            &mut highlighter,
                            DiffKind::Removed,
                        ),
                        DiffSide::New => side_line(" ", None, "", width, Some(Color::DarkGray)),
                    },
                });
            }

            lines.push(Line::default());
        }
        lines.push(Line::default());
    }

    lines.into_iter().skip(scroll as usize).collect()
}

fn file_header_line(file: &DiffFile, selected: bool) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD)
    };

    Line::from(Span::styled(
        format!("diff -- {} -> {}", file.old_path, file.new_path),
        style,
    ))
}

fn file_side_header_line(file: &DiffFile, selected: bool, side: DiffSide) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD)
    };

    let label = match side {
        DiffSide::Old => file.old_path.as_str(),
        DiffSide::New => file.new_path.as_str(),
    };

    Line::from(Span::styled(format!("diff -- {}", label), style))
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

fn file_separator_line(width: usize) -> Line<'static> {
    let separator = "─".repeat(width.max(8));
    Line::from(Span::styled(
        separator,
        Style::default().fg(Color::DarkGray),
    ))
}

fn hunk_header_line<'a>(header: &'a str, selected: bool) -> Line<'a> {
    let style = if selected {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    Line::from(Span::styled(header, style))
}

fn highlighted_prefixed_line(
    prefix: &str,
    old_lineno: Option<usize>,
    new_lineno: Option<usize>,
    text: &str,
    color: Option<Color>,
    width: usize,
    highlighter: &mut FileHighlighter<'static>,
    diff_kind: DiffKind,
) -> Line<'static> {
    let background = diff_background(diff_kind);
    let style = color
        .map(|value| Style::default().fg(value))
        .map(|style| match background {
            Some(background) => style.bg(background),
            None => style,
        })
        .unwrap_or_default();
    let line_number_style = match background {
        Some(background) => Style::default().fg(Color::DarkGray).bg(background),
        None => Style::default().fg(Color::DarkGray),
    };
    let mut spans = vec![
        Span::styled(format!("{prefix:>1} "), style),
        Span::styled(
            format!("{:>4} ", format_lineno(old_lineno)),
            line_number_style,
        ),
        Span::styled(
            format!("{:>4} ", format_lineno(new_lineno)),
            line_number_style,
        ),
    ];
    let prefix_width: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    let available_width = width.saturating_sub(prefix_width);
    let mut code_spans = highlighter.highlight_line(text, diff_kind);
    let rendered_code_width: usize = code_spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum();
    if rendered_code_width > available_width {
        code_spans = vec![Span::styled(
            truncate_text(text, available_width),
            background
                .map(|value| Style::default().bg(value))
                .unwrap_or_default(),
        )];
    } else if rendered_code_width < available_width {
        code_spans.push(Span::styled(
            " ".repeat(available_width - rendered_code_width),
            background
                .map(|value| Style::default().bg(value))
                .unwrap_or_default(),
        ));
    }
    spans.extend(code_spans);
    let rendered_width: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    if let Some(background) = background {
        if rendered_width < width {
            spans.push(Span::styled(
                " ".repeat(width - rendered_width),
                Style::default().bg(background),
            ));
        }
    }
    Line::from(spans)
}

fn side_line<'a>(
    prefix: &'a str,
    lineno: Option<usize>,
    text: &'a str,
    width: usize,
    color: Option<Color>,
) -> Line<'a> {
    let style = color
        .map(|value| Style::default().fg(value))
        .unwrap_or_default();
    let body = format!(
        "{:>4} {} {}",
        format_lineno(lineno),
        prefix,
        truncate_text(text, width.saturating_sub(8))
    );

    Line::from(Span::styled(pad_to_width(&body, width), style))
}

fn highlighted_side_line(
    prefix: &str,
    lineno: Option<usize>,
    text: &str,
    width: usize,
    color: Option<Color>,
    highlighter: &mut FileHighlighter<'static>,
    diff_kind: DiffKind,
) -> Line<'static> {
    let background = diff_background(diff_kind);
    let prefix_style = color
        .map(|value| Style::default().fg(value))
        .map(|style| match background {
            Some(background) => style.bg(background),
            None => style,
        })
        .unwrap_or_default();
    let line_number = format!("{:>4} {} ", format_lineno(lineno), prefix);
    let available_width = width.saturating_sub(line_number.chars().count());
    let mut spans = vec![Span::styled(
        pad_to_width(&line_number, line_number.chars().count()),
        prefix_style,
    )];
    let mut code_spans = highlighter.highlight_line(text, diff_kind);

    let rendered_code_width: usize = code_spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum();
    if rendered_code_width > available_width {
        code_spans = vec![Span::styled(
            truncate_text(text, available_width),
            background
                .map(|value| Style::default().bg(value))
                .unwrap_or_default(),
        )];
    } else if rendered_code_width < available_width {
        code_spans.push(Span::styled(
            " ".repeat(available_width - rendered_code_width),
            background
                .map(|value| Style::default().bg(value))
                .unwrap_or_default(),
        ));
    }

    spans.extend(code_spans);
    Line::from(spans)
}

fn diff_background(diff_kind: DiffKind) -> Option<Color> {
    match diff_kind {
        DiffKind::Context => None,
        DiffKind::Added => Some(Color::Rgb(18, 48, 24)),
        DiffKind::Removed => Some(Color::Rgb(60, 24, 24)),
    }
}

fn truncate_text(text: &str, max_width: usize) -> String {
    if text.chars().count() <= max_width {
        return text.to_string();
    }

    if max_width <= 1 {
        return "…".to_string();
    }

    let mut truncated = String::new();
    for ch in text.chars().take(max_width - 1) {
        truncated.push(ch);
    }
    truncated.push('…');
    truncated
}

fn pad_to_width(text: &str, width: usize) -> String {
    let current = text.chars().count();
    if current >= width {
        return text.to_string();
    }

    format!("{text}{:width$}", "", width = width - current)
}

fn format_lineno(lineno: Option<usize>) -> String {
    lineno
        .map(|value| value.to_string())
        .unwrap_or_else(|| "·".to_string())
}
