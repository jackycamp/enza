mod loader;
mod ui;

use crate::diff::{DiffFilter, DiffTarget};

pub use ui::run_landing_page;

pub struct LandingSelection {
    pub target: DiffTarget,
    pub diff_filter: Option<DiffFilter>,
}
