//! Diff layout pipeline.
//!
//! The source diff lives in `DiffSession`; layout should not duplicate that data.
//! `plan` records where each file and hunk starts, `window` decides which hunks
//! have rendered rows loaded, and `lines` renders only the rows visible on
//! screen. Notes are inserted before base rows instead of changing the base row
//! map. Keep these layers separate so unloaded hunks still keep stable row
//! numbers without storing rendered rows for the whole diff.

mod access;
mod build;
mod lines;
mod model;
mod notes;
mod plan;
#[cfg(test)]
mod tests;
mod text;
mod window;
mod worker;

pub use model::{Layout, NodeStatus, RowContext, RowKind, RowViewState};
pub(crate) use window::{HunkWindowTarget, LayoutBuildOptions, LayoutWidths};
pub use worker::LayoutWorker;
