//! Row rendering helpers.

mod diff_lines;
mod headers;
mod materialize;

pub use headers::{
    file_header_line, file_header_row, file_separator_line, file_side_by_side_header_line,
    hunk_header_line, hunk_header_row, side_by_side_hunk_header_line,
};
pub use diff_lines::{
    build_combined_side_line, build_inline_line, combined_side_line, split_side_by_side_width,
};
