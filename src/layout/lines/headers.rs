use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::diff::DiffFile;
use crate::layout::model::RenderRow;
use crate::layout::text::fit_text;

use super::diff_lines::split_side_by_side_width;

/// Builds the horizontal separator row before a file header.
pub fn file_separator_line(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(8)),
        Style::default().fg(Color::DarkGray),
    ))
}

/// Wraps normal and selected file header lines in a selectable render row.
pub fn file_header_row(
    file_index: usize,
    normal: Line<'static>,
    selected: Line<'static>,
) -> RenderRow {
    RenderRow::FileHeader {
        file_index,
        normal,
        selected,
    }
}

/// Wraps normal and selected hunk header lines in a selectable render row.
pub fn hunk_header_row(
    file_index: usize,
    hunk_index: usize,
    normal: Line<'static>,
    selected: Line<'static>,
) -> RenderRow {
    RenderRow::HunkHeader {
        file_index,
        hunk_index,
        normal,
        selected,
    }
}

/// Builds the inline-mode file header line.
pub fn file_header_line(file: &DiffFile, selected: bool, width: usize) -> Line<'static> {
    let label = if file.new_path != "/dev/null" {
        file.new_path.as_str()
    } else {
        file.old_path.as_str()
    };

    header_line(width, label, file, selected)
}

/// Builds the side-by-side-mode file header line.
pub fn file_side_by_side_header_line(
    file: &DiffFile,
    selected: bool,
    width: usize,
) -> Line<'static> {
    header_line(width, &file.path, file, selected)
}

/// Builds the side-by-side hunk header with the center gutter preserved.
pub fn side_by_side_hunk_header_line(header: &str, selected: bool, width: usize) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let divider_style = Style::default().fg(Color::DarkGray);
    let (left_width, right_width) = split_side_by_side_width(width);
    let left = fit_text(header, left_width);
    let right = fit_text("", right_width);

    Line::from(vec![
        Span::styled(left, style),
        Span::styled(" │ ".to_string(), divider_style),
        Span::styled(right, style),
    ])
}

/// Builds the inline-mode hunk header line.
pub fn hunk_header_line(header: &str, selected: bool) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    Line::from(Span::styled(header.to_string(), style))
}

/// Builds the shared file header line with file label and change counts.
fn header_line(width: usize, label: &str, file: &DiffFile, selected: bool) -> Line<'static> {
    let (additions, deletions) = file.change_counts();
    let title_style = if selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD)
    };

    let additions_style = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let deletions_style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let divider_style = Style::default().fg(Color::DarkGray);

    let suffix = format!("+{additions}, -{deletions}");
    let available_label_width = width.saturating_sub(suffix.chars().count());
    let label = fit_text(&format!(" {label}"), available_label_width.max(1))
        .trim_end()
        .to_string();
    let rendered_width = label.chars().count() + suffix.chars().count();
    let trailing = " ".repeat(width.saturating_sub(rendered_width));

    Line::from(vec![
        Span::styled(label, title_style),
        Span::styled("  ".to_string(), divider_style),
        Span::styled(format!("+{additions}"), additions_style),
        Span::styled(", ".to_string(), divider_style),
        Span::styled(format!("-{deletions}"), deletions_style),
        Span::styled(trailing, divider_style),
    ])
}
