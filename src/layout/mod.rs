//! Diff layout pipeline.
//!
//! The source diff lives in `DiffSession`; layout should not duplicate that data.
//! `plan` builds a compact logical row map from file/hunk spans, `window` decides
//! which hunks should have rendered rows resident, and `lines` materializes only
//! the rows visible in the current viewport. Notes are stored as overlay
//! insertions on top of the base plan. Keep these layers separate so unloaded
//! hunks still have stable logical row positions without forcing all rows to be
//! rendered or stored in `Layout`.

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
